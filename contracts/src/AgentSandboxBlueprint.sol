// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./OperatorSelection.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";

/**
 * @title AgentSandboxBlueprint
 * @dev Unified service manager for AI Agent Sandbox Blueprint.
 *      Deployed 3x with different mode flags:
 *        - Cloud mode (instanceMode=false): Multi-operator fleet with capacity-weighted
 *          sandbox assignment, workflow storage, and batch operations.
 *        - Instance mode (instanceMode=true): Per-service singleton sandbox with
 *          operator self-provisioning. Config stored at service request time.
 *        - TEE instance mode (instanceMode=true, teeRequired=true): Same as instance
 *          but requires TEE attestation on provision.
 *
 *      7 on-chain jobs (state-changing only). All read-only operations (exec, prompt,
 *      task, stop, resume, snapshot, SSH) are served via the operator HTTP API.
 */
contract AgentSandboxBlueprint is OperatorSelectionBase {
    // ═══════════════════════════════════════════════════════════════════════════
    // JOB IDS (7 total — state-changing only)
    // ═══════════════════════════════════════════════════════════════════════════

    uint8 public constant JOB_SANDBOX_CREATE = 0;
    uint8 public constant JOB_SANDBOX_DELETE = 1;
    uint8 public constant JOB_WORKFLOW_CREATE = 2;
    uint8 public constant JOB_WORKFLOW_TRIGGER = 3;
    uint8 public constant JOB_WORKFLOW_CANCEL = 4;
    uint8 public constant JOB_PROVISION = 5;      // Instance mode only
    uint8 public constant JOB_DEPROVISION = 6;    // Instance mode only

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    string public constant BLUEPRINT_NAME = "ai-agent-sandbox-blueprint";
    string public constant BLUEPRINT_VERSION = "0.4.0";

    // ═══════════════════════════════════════════════════════════════════════════
    // MODE FLAGS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice When true, this deployment operates in instance mode:
    ///         one sandbox per service, operators self-provision.
    bool public instanceMode;

    /// @notice When true, provision requires non-empty TEE attestation.
    bool public teeRequired;

    // ═══════════════════════════════════════════════════════════════════════════
    // PER-JOB PRICING MULTIPLIERS
    // ═══════════════════════════════════════════════════════════════════════════

    // Cloud jobs
    uint256 public constant PRICE_MULT_SANDBOX_CREATE = 50;
    uint256 public constant PRICE_MULT_SANDBOX_DELETE = 1;
    uint256 public constant PRICE_MULT_WORKFLOW_CREATE = 2;
    uint256 public constant PRICE_MULT_WORKFLOW_TRIGGER = 5;
    uint256 public constant PRICE_MULT_WORKFLOW_CANCEL = 1;

    // Instance jobs
    uint256 public constant PRICE_MULT_PROVISION = 50;
    uint256 public constant PRICE_MULT_DEPROVISION = 1;

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR CAPACITY STATE (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    mapping(address => uint32) public operatorMaxCapacity;
    mapping(address => uint32) public operatorActiveSandboxes;
    uint32 public defaultMaxCapacity = 100;
    uint32 public totalActiveSandboxes;

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR ASSIGNMENT STATE (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    mapping(uint64 => mapping(uint64 => address)) internal _createAssignments;
    uint256 internal _selectionNonce;

    // ═══════════════════════════════════════════════════════════════════════════
    // SANDBOX REGISTRY (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    mapping(bytes32 => address) public sandboxOperator;
    mapping(bytes32 => bool) public sandboxActive;

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW STATE (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    struct WorkflowCreateRequest {
        string name;
        string workflow_json;
        string trigger_type;
        string trigger_config;
        string sandbox_config_json;
    }

    struct WorkflowControlRequest {
        uint64 workflow_id;
    }

    struct WorkflowConfig {
        string name;
        string workflow_json;
        string trigger_type;
        string trigger_config;
        string sandbox_config_json;
        bool active;
        uint64 created_at;
        uint64 updated_at;
        uint64 last_triggered_at;
    }

    mapping(uint64 => WorkflowConfig) private workflows;
    mapping(uint64 => uint256) private workflow_index;
    uint64[] private workflow_ids;

    // ═══════════════════════════════════════════════════════════════════════════
    // INSTANCE STATE (instance mode)
    // ═══════════════════════════════════════════════════════════════════════════

    mapping(uint64 => uint32) public instanceOperatorCount;
    mapping(uint64 => mapping(address => bool)) public operatorProvisioned;
    mapping(uint64 => mapping(address => bytes32)) public operatorAttestationHash;
    mapping(uint64 => address[]) internal _serviceOperators;
    mapping(uint64 => mapping(address => uint256)) internal _operatorIndex;
    mapping(uint64 => mapping(address => string)) public operatorSidecarUrl;

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE CONFIG STORAGE (instance mode)
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Sandbox config submitted at service request time, keyed by requestId.
    ///         Moved to serviceConfig[serviceId] in onServiceInitialized.
    mapping(uint64 => bytes) internal _pendingRequestConfig;

    /// @notice Sandbox config keyed by serviceId, set after service initialization.
    mapping(uint64 => bytes) public serviceConfig;

    /// @notice Service requester address, set during onServiceInitialized.
    ///         Used by operators to assign sandbox ownership.
    mapping(uint64 => address) public serviceOwner;

    // ═══════════════════════════════════════════════════════════════════════════
    // EVENTS
    // ═══════════════════════════════════════════════════════════════════════════

    // Cloud events
    event OperatorAssigned(uint64 indexed serviceId, uint64 indexed callId, address indexed operator);
    event OperatorRouted(uint64 indexed serviceId, uint64 indexed callId, address indexed operator);
    event SandboxCreated(bytes32 indexed sandboxHash, address indexed operator);
    event SandboxDeleted(bytes32 indexed sandboxHash, address indexed operator);
    event WorkflowStored(uint64 indexed workflow_id, string trigger_type, string trigger_config);
    event WorkflowTriggered(uint64 indexed workflow_id, uint64 triggered_at);
    event WorkflowCanceled(uint64 indexed workflow_id, uint64 canceled_at);

    // Instance events
    event OperatorProvisioned(uint64 indexed serviceId, address indexed operator, string sandboxId, string sidecarUrl);
    event OperatorDeprovisioned(uint64 indexed serviceId, address indexed operator);
    event TeeAttestationStored(uint64 indexed serviceId, address indexed operator, bytes32 attestationHash);
    event ServiceTerminationReceived(uint64 indexed serviceId, address indexed owner);
    event ServiceConfigStored(uint64 indexed serviceId, uint64 indexed requestId);

    // ═══════════════════════════════════════════════════════════════════════════
    // ERRORS
    // ═══════════════════════════════════════════════════════════════════════════

    // Cloud errors
    error NoAvailableCapacity();
    error OperatorMismatch(address expected, address actual);
    error SandboxNotFound(bytes32 sandboxHash);
    error SandboxAlreadyExists(bytes32 sandboxHash);

    // Instance errors
    error AlreadyProvisioned(uint64 serviceId, address operator);
    error NotProvisioned(uint64 serviceId, address operator);
    error MissingTeeAttestation(uint64 serviceId, address operator);

    // ═══════════════════════════════════════════════════════════════════════════
    // CONSTRUCTOR
    // ═══════════════════════════════════════════════════════════════════════════

    constructor(address restakingAddress, bool _instanceMode, bool _teeRequired) {
        if (restakingAddress != address(0)) {
            restaking = IMultiAssetDelegation(restakingAddress);
        }
        instanceMode = _instanceMode;
        teeRequired = _teeRequired;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function jobIds() external pure returns (uint8[] memory ids) {
        ids = new uint8[](7);
        ids[0] = JOB_SANDBOX_CREATE;
        ids[1] = JOB_SANDBOX_DELETE;
        ids[2] = JOB_WORKFLOW_CREATE;
        ids[3] = JOB_WORKFLOW_TRIGGER;
        ids[4] = JOB_WORKFLOW_CANCEL;
        ids[5] = JOB_PROVISION;
        ids[6] = JOB_DEPROVISION;
    }

    function supportsJob(uint8 jobId) external pure returns (bool) {
        return jobId <= JOB_DEPROVISION;
    }

    function jobCount() external pure returns (uint256) {
        return 7;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR REGISTRATION
    // ═══════════════════════════════════════════════════════════════════════════

    function onRegister(
        address operator,
        bytes calldata registrationInputs
    ) external payable override onlyFromTangle {
        if (!instanceMode) {
            uint32 capacity = defaultMaxCapacity;
            if (registrationInputs.length >= 32) {
                uint32 decoded = abi.decode(registrationInputs, (uint32));
                if (decoded > 0) {
                    capacity = decoded;
                }
            }
            operatorMaxCapacity[operator] = capacity;
        }
        // Instance mode: no-op (operators self-provision via JOB_PROVISION)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE REQUEST VALIDATION
    // ═══════════════════════════════════════════════════════════════════════════

    function onRequest(
        uint64 requestId,
        address requester,
        address[] calldata operators,
        bytes calldata requestInputs,
        uint64 ttl,
        address paymentAsset,
        uint256 paymentAmount
    ) external payable override onlyFromTangle {
        requester;
        ttl;
        paymentAsset;
        paymentAmount;

        require(operators.length >= 1, "At least 1 operator required");

        if (instanceMode) {
            // Store sandbox config for retrieval in onServiceInitialized
            if (requestInputs.length > 0) {
                _pendingRequestConfig[requestId] = requestInputs;
            }
        } else {
            SelectionRequest memory selection = _decodeSelectionRequest(requestInputs);
            _validateOperatorSelection(operators, selection);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE LIFECYCLE HOOKS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Called when the service is initialized. Moves pending config
    ///         to persistent storage keyed by serviceId.
    function onServiceInitialized(
        uint64,              // blueprintId
        uint64 requestId,
        uint64 serviceId,
        address owner,
        address[] calldata,  // permittedCallers
        uint64               // ttl
    ) external override onlyFromTangle {
        if (instanceMode) {
            serviceOwner[serviceId] = owner;
            bytes memory cfg = _pendingRequestConfig[requestId];
            if (cfg.length > 0) {
                serviceConfig[serviceId] = cfg;
                delete _pendingRequestConfig[requestId];
                emit ServiceConfigStored(serviceId, requestId);
            }
        }
    }

    /// @notice Called when the service owner terminates the service.
    function onServiceTermination(
        uint64 serviceId,
        address owner
    ) external override onlyFromTangle {
        emit ServiceTerminationReceived(serviceId, owner);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB CALL HOOK — OPERATOR ASSIGNMENT & ROUTING
    // ═══════════════════════════════════════════════════════════════════════════

    function onJobCall(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        bytes calldata inputs
    ) external payable override onlyFromTangle {
        if (job == JOB_SANDBOX_CREATE) {
            address selected = _selectByCapacity(serviceId);
            _createAssignments[serviceId][jobCallId] = selected;
            emit OperatorAssigned(serviceId, jobCallId, selected);
        } else if (job == JOB_SANDBOX_DELETE) {
            string memory sandboxId = abi.decode(inputs, (string));
            bytes32 sandboxHash = keccak256(bytes(sandboxId));
            address routed = sandboxOperator[sandboxHash];
            if (routed == address(0)) revert SandboxNotFound(sandboxHash);
            emit OperatorRouted(serviceId, jobCallId, routed);
        }
        // Workflow and instance jobs: no on-chain routing needed.
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB RESULT HOOK — STATE UPDATES
    // ═══════════════════════════════════════════════════════════════════════════

    function onJobResult(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        address operator,
        bytes calldata inputs,
        bytes calldata outputs
    ) external payable override onlyFromTangle {
        if (job == JOB_SANDBOX_CREATE) {
            _handleCreateResult(serviceId, jobCallId, operator, outputs);
        } else if (job == JOB_SANDBOX_DELETE) {
            _handleDeleteResult(operator, inputs);
        } else if (job == JOB_WORKFLOW_CREATE) {
            WorkflowCreateRequest memory request = abi.decode(inputs, (WorkflowCreateRequest));
            _upsert_workflow(jobCallId, request);
        } else if (job == JOB_WORKFLOW_TRIGGER) {
            WorkflowControlRequest memory request = abi.decode(inputs, (WorkflowControlRequest));
            _mark_triggered(request.workflow_id);
        } else if (job == JOB_WORKFLOW_CANCEL) {
            WorkflowControlRequest memory request = abi.decode(inputs, (WorkflowControlRequest));
            _cancel_workflow(request.workflow_id);
        } else if (job == JOB_PROVISION) {
            _handleProvisionResult(serviceId, operator, outputs);
        } else if (job == JOB_DEPROVISION) {
            _handleDeprovisionResult(serviceId, operator);
        }
    }

    function getRequiredResultCount(uint64, uint8) external pure override returns (uint32) {
        return 1;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // ADMIN FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════════════

    function setDefaultMaxCapacity(uint32 capacity) external onlyBlueprintOwner {
        defaultMaxCapacity = capacity;
    }

    function setOperatorCapacity(address operator, uint32 capacity) external onlyBlueprintOwner {
        operatorMaxCapacity[operator] = capacity;
    }

    function setInstanceMode(bool _mode) external onlyBlueprintOwner {
        instanceMode = _mode;
    }

    function setTeeRequired(bool _required) external onlyBlueprintOwner {
        teeRequired = _required;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS — CLOUD MODE
    // ═══════════════════════════════════════════════════════════════════════════

    function getOperatorLoad(address operator) external view returns (uint32 active, uint32 max) {
        return (operatorActiveSandboxes[operator], operatorMaxCapacity[operator]);
    }

    function getSandboxOperator(string calldata sandboxId) external view returns (address) {
        return sandboxOperator[keccak256(bytes(sandboxId))];
    }

    function isSandboxActive(string calldata sandboxId) external view returns (bool) {
        return sandboxActive[keccak256(bytes(sandboxId))];
    }

    function getAvailableCapacity() external view returns (uint32 available) {
        if (address(restaking) == address(0)) return 0;
        uint256 total = restaking.operatorCount();
        for (uint256 i = 0; i < total; i++) {
            address op = restaking.operatorAt(i);
            if (_isEligibleOperator(op)) {
                uint32 max = operatorMaxCapacity[op];
                uint32 active = operatorActiveSandboxes[op];
                if (max > active) {
                    available += (max - active);
                }
            }
        }
    }

    function getServiceStats() external view returns (uint32 totalSandboxes, uint32 totalCapacity) {
        totalSandboxes = totalActiveSandboxes;
        if (address(restaking) == address(0)) return (totalSandboxes, 0);
        uint256 total = restaking.operatorCount();
        for (uint256 i = 0; i < total; i++) {
            address op = restaking.operatorAt(i);
            if (_isEligibleOperator(op)) {
                totalCapacity += operatorMaxCapacity[op];
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS — INSTANCE MODE
    // ═══════════════════════════════════════════════════════════════════════════

    function isProvisioned(uint64 serviceId) external view returns (bool) {
        return instanceOperatorCount[serviceId] > 0;
    }

    function isOperatorProvisioned(uint64 serviceId, address operator) external view returns (bool) {
        return operatorProvisioned[serviceId][operator];
    }

    function getOperatorCount(uint64 serviceId) external view returns (uint32) {
        return instanceOperatorCount[serviceId];
    }

    function getAttestationHash(uint64 serviceId, address operator) external view returns (bytes32) {
        return operatorAttestationHash[serviceId][operator];
    }

    function getOperatorEndpoints(uint64 serviceId)
        external
        view
        returns (address[] memory operators, string[] memory sidecarUrls)
    {
        operators = _serviceOperators[serviceId];
        sidecarUrls = new string[](operators.length);
        for (uint256 i = 0; i < operators.length; i++) {
            sidecarUrls[i] = operatorSidecarUrl[serviceId][operators[i]];
        }
    }

    function getServiceConfig(uint64 serviceId) external view returns (bytes memory) {
        return serviceConfig[serviceId];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW VIEWS
    // ═══════════════════════════════════════════════════════════════════════════

    function getWorkflow(uint64 workflowId) external view returns (WorkflowConfig memory) {
        return workflows[workflowId];
    }

    function getWorkflowIds(bool activeOnly) external view returns (uint64[] memory ids) {
        if (!activeOnly) {
            return workflow_ids;
        }

        uint256 total = workflow_ids.length;
        uint256 count = 0;
        for (uint256 i = 0; i < total; i++) {
            if (workflows[workflow_ids[i]].active) {
                count++;
            }
        }

        ids = new uint64[](count);
        uint256 idx = 0;
        for (uint256 i = 0; i < total; i++) {
            uint64 workflowId = workflow_ids[i];
            if (workflows[workflowId].active) {
                ids[idx] = workflowId;
                idx++;
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PRICING HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    function getDefaultJobRates(uint256 baseRate)
        external
        pure
        returns (uint8[] memory jobIndexes, uint256[] memory rates)
    {
        jobIndexes = new uint8[](7);
        rates = new uint256[](7);

        jobIndexes[0] = JOB_SANDBOX_CREATE;    rates[0] = baseRate * PRICE_MULT_SANDBOX_CREATE;
        jobIndexes[1] = JOB_SANDBOX_DELETE;    rates[1] = baseRate * PRICE_MULT_SANDBOX_DELETE;
        jobIndexes[2] = JOB_WORKFLOW_CREATE;   rates[2] = baseRate * PRICE_MULT_WORKFLOW_CREATE;
        jobIndexes[3] = JOB_WORKFLOW_TRIGGER;  rates[3] = baseRate * PRICE_MULT_WORKFLOW_TRIGGER;
        jobIndexes[4] = JOB_WORKFLOW_CANCEL;   rates[4] = baseRate * PRICE_MULT_WORKFLOW_CANCEL;
        jobIndexes[5] = JOB_PROVISION;         rates[5] = baseRate * PRICE_MULT_PROVISION;
        jobIndexes[6] = JOB_DEPROVISION;       rates[6] = baseRate * PRICE_MULT_DEPROVISION;
    }

    function getJobPriceMultiplier(uint8 jobId) external pure returns (uint256) {
        if (jobId == JOB_SANDBOX_CREATE)    return PRICE_MULT_SANDBOX_CREATE;
        if (jobId == JOB_SANDBOX_DELETE)    return PRICE_MULT_SANDBOX_DELETE;
        if (jobId == JOB_WORKFLOW_CREATE)   return PRICE_MULT_WORKFLOW_CREATE;
        if (jobId == JOB_WORKFLOW_TRIGGER)  return PRICE_MULT_WORKFLOW_TRIGGER;
        if (jobId == JOB_WORKFLOW_CANCEL)   return PRICE_MULT_WORKFLOW_CANCEL;
        if (jobId == JOB_PROVISION)         return PRICE_MULT_PROVISION;
        if (jobId == JOB_DEPROVISION)       return PRICE_MULT_DEPROVISION;
        return 0;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: CAPACITY-WEIGHTED OPERATOR SELECTION (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function _selectByCapacity(uint64 serviceId) internal returns (address) {
        if (address(restaking) == address(0)) revert RestakingNotSet();

        uint256 total = restaking.operatorCount();
        address[] memory candidates = new address[](total);
        uint32[] memory weights = new uint32[](total);
        uint32 totalWeight = 0;
        uint256 count = 0;

        for (uint256 i = 0; i < total; i++) {
            address op = restaking.operatorAt(i);
            if (!_isEligibleOperator(op)) continue;
            uint32 max = operatorMaxCapacity[op];
            uint32 active = operatorActiveSandboxes[op];
            if (max <= active) continue;
            uint32 weight = max - active;
            candidates[count] = op;
            weights[count] = weight;
            totalWeight += weight;
            count++;
        }

        if (count == 0 || totalWeight == 0) revert NoAvailableCapacity();

        uint256 rand = uint256(keccak256(abi.encode(block.prevrandao, serviceId, _selectionNonce)));
        _selectionNonce++;
        uint256 pick = rand % totalWeight;

        uint32 cumulative = 0;
        for (uint256 i = 0; i < count; i++) {
            cumulative += weights[i];
            if (pick < cumulative) {
                return candidates[i];
            }
        }

        return candidates[count - 1];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: SANDBOX CREATE/DELETE RESULT HANDLING (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function _handleCreateResult(
        uint64 serviceId,
        uint64 jobCallId,
        address operator,
        bytes calldata outputs
    ) internal {
        address assigned = _createAssignments[serviceId][jobCallId];
        if (assigned != operator) revert OperatorMismatch(assigned, operator);

        (string memory sandboxId,) = abi.decode(outputs, (string, string));
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        if (sandboxOperator[sandboxHash] != address(0)) revert SandboxAlreadyExists(sandboxHash);

        sandboxOperator[sandboxHash] = operator;
        sandboxActive[sandboxHash] = true;
        operatorActiveSandboxes[operator]++;
        totalActiveSandboxes++;

        delete _createAssignments[serviceId][jobCallId];

        emit SandboxCreated(sandboxHash, operator);
    }

    function _handleDeleteResult(address operator, bytes calldata inputs) internal {
        string memory sandboxId = abi.decode(inputs, (string));
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        address expected = sandboxOperator[sandboxHash];
        if (expected == address(0)) revert SandboxNotFound(sandboxHash);
        if (expected != operator) revert OperatorMismatch(expected, operator);

        delete sandboxOperator[sandboxHash];
        sandboxActive[sandboxHash] = false;
        operatorActiveSandboxes[operator]--;
        totalActiveSandboxes--;

        emit SandboxDeleted(sandboxHash, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: PROVISION/DEPROVISION (instance mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function _handleProvisionResult(
        uint64 serviceId,
        address operator,
        bytes calldata outputs
    ) internal {
        if (operatorProvisioned[serviceId][operator]) revert AlreadyProvisioned(serviceId, operator);

        (string memory sandboxId, string memory sidecarUrl,, string memory teeAttestationJson) =
            abi.decode(outputs, (string, string, uint32, string));

        // TEE attestation enforcement
        if (teeRequired) {
            if (bytes(teeAttestationJson).length == 0) {
                revert MissingTeeAttestation(serviceId, operator);
            }
        }

        operatorProvisioned[serviceId][operator] = true;
        instanceOperatorCount[serviceId]++;

        operatorSidecarUrl[serviceId][operator] = sidecarUrl;
        _serviceOperators[serviceId].push(operator);
        _operatorIndex[serviceId][operator] = _serviceOperators[serviceId].length; // 1-indexed

        // Store TEE attestation hash if present
        if (bytes(teeAttestationJson).length > 0) {
            bytes32 attestationHash = keccak256(bytes(teeAttestationJson));
            operatorAttestationHash[serviceId][operator] = attestationHash;
            emit TeeAttestationStored(serviceId, operator, attestationHash);
        }

        emit OperatorProvisioned(serviceId, operator, sandboxId, sidecarUrl);
    }

    function _handleDeprovisionResult(
        uint64 serviceId,
        address operator
    ) internal {
        if (!operatorProvisioned[serviceId][operator]) revert NotProvisioned(serviceId, operator);

        operatorProvisioned[serviceId][operator] = false;
        instanceOperatorCount[serviceId]--;

        // Swap-and-pop to remove operator from enumerable list
        uint256 index = _operatorIndex[serviceId][operator];
        if (index > 0) {
            uint256 lastIndex = _serviceOperators[serviceId].length;
            if (index != lastIndex) {
                address lastOperator = _serviceOperators[serviceId][lastIndex - 1];
                _serviceOperators[serviceId][index - 1] = lastOperator;
                _operatorIndex[serviceId][lastOperator] = index;
            }
            _serviceOperators[serviceId].pop();
            delete _operatorIndex[serviceId][operator];
        }

        delete operatorSidecarUrl[serviceId][operator];
        delete operatorAttestationHash[serviceId][operator];

        emit OperatorDeprovisioned(serviceId, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: WORKFLOW STORAGE (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function _upsert_workflow(uint64 workflowId, WorkflowCreateRequest memory request) internal {
        WorkflowConfig storage config = workflows[workflowId];
        if (workflow_index[workflowId] == 0) {
            workflow_ids.push(workflowId);
            workflow_index[workflowId] = workflow_ids.length;
            config.created_at = uint64(block.timestamp);
        }

        config.name = request.name;
        config.workflow_json = request.workflow_json;
        config.trigger_type = request.trigger_type;
        config.trigger_config = request.trigger_config;
        config.sandbox_config_json = request.sandbox_config_json;
        config.active = true;
        config.updated_at = uint64(block.timestamp);

        emit WorkflowStored(workflowId, request.trigger_type, request.trigger_config);
    }

    function _mark_triggered(uint64 workflowId) internal {
        WorkflowConfig storage config = workflows[workflowId];
        if (workflow_index[workflowId] == 0) {
            return;
        }
        config.last_triggered_at = uint64(block.timestamp);
        config.updated_at = uint64(block.timestamp);
        emit WorkflowTriggered(workflowId, uint64(block.timestamp));
    }

    function _cancel_workflow(uint64 workflowId) internal {
        WorkflowConfig storage config = workflows[workflowId];
        if (workflow_index[workflowId] == 0) {
            return;
        }
        config.active = false;
        config.updated_at = uint64(block.timestamp);
        emit WorkflowCanceled(workflowId, uint64(block.timestamp));
    }
}
