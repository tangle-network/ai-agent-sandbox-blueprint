// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./OperatorSelection.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";

interface ITangleServiceOperatorView {
    function isServiceOperator(uint64 serviceId, address operator) external view returns (bool);
}

/**
 * @title AgentSandboxBlueprint
 * @dev Unified service manager for AI Agent Sandbox Blueprint.
 *      Deployed 3x with different mode flags:
 *        - Cloud mode (instanceMode=false): Multi-operator fleet with capacity-weighted
 *          sandbox assignment and workflow storage.
 *        - Instance mode (instanceMode=true): Per-service singleton sandbox with
 *          operator self-provisioning plus workflow support.
 *        - TEE instance mode (instanceMode=true, teeRequired=true): Same as instance
 *          but requires TEE attestation on provision.
 *
 *      5 on-chain jobs (state-changing only). All read-only operations (exec, prompt,
 *      task, stop, resume, snapshot, SSH) are served via the operator HTTP API.
 */
contract AgentSandboxBlueprint is OperatorSelectionBase {
    // ═══════════════════════════════════════════════════════════════════════════
    // JOB IDS (5 total — state-changing only)
    // ═══════════════════════════════════════════════════════════════════════════

    uint8 public constant JOB_SANDBOX_CREATE = 0;
    uint8 public constant JOB_SANDBOX_DELETE = 1;
    uint8 public constant JOB_WORKFLOW_CREATE = 2;
    uint8 public constant JOB_WORKFLOW_TRIGGER = 3;
    uint8 public constant JOB_WORKFLOW_CANCEL = 4;

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

    // ═══════════════════════════════════════════════════════════════════════════
    // ARRAY BOUNDS (storage griefing prevention)
    // ═══════════════════════════════════════════════════════════════════════════

    uint256 public constant MAX_WORKFLOWS = 10000;
    uint32 public constant MAX_OPERATORS_PER_SERVICE = 1000;

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

    struct SandboxIdRequest {
        string sandbox_id;
    }

    struct SandboxCreateOutput {
        string sandboxId;
        string json;
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
    uint256 public totalProvisionedOperators;

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

    // Request validation events
    event ServiceRequestValidated(uint64 indexed requestId, address requester, uint32 operatorCount);

    // ═══════════════════════════════════════════════════════════════════════════
    // ERRORS
    // ═══════════════════════════════════════════════════════════════════════════

    // Cloud errors
    error NoAvailableCapacity();
    error OperatorMismatch(address expected, address actual);
    error SandboxNotFound(bytes32 sandboxHash);
    error SandboxAlreadyExists(bytes32 sandboxHash);
    error EmptySandboxId();
    error SandboxIdTooLong(uint256 length);
    error WorkflowNotFound(uint64 workflowId);
    error MaxWorkflowsReached(uint64 serviceId);

    // Instance errors
    error AlreadyProvisioned(uint64 serviceId, address operator);
    error NotProvisioned(uint64 serviceId, address operator);
    error MissingTeeAttestation(uint64 serviceId, address operator);
    error MaxOperatorsReached(uint64 serviceId);
    error OperatorNotInService(uint64 serviceId, address operator);

    // Mode errors
    error CloudModeOnly();
    error InstanceModeOnly();
    error UnknownJobId(uint8 jobId);
    error CannotChangeWithActiveResources();
    error CannotLeaveWithActiveResources();
    error ZeroOperatorsInRequest();
    error ServiceAlreadyInitialized(uint64 serviceId);

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

    /// @notice Returns all supported job IDs for this blueprint.
    function jobIds() external pure returns (uint8[] memory ids) {
        ids = new uint8[](5);
        ids[0] = JOB_SANDBOX_CREATE;
        ids[1] = JOB_SANDBOX_DELETE;
        ids[2] = JOB_WORKFLOW_CREATE;
        ids[3] = JOB_WORKFLOW_TRIGGER;
        ids[4] = JOB_WORKFLOW_CANCEL;
    }

    /// @notice Returns true if this blueprint supports the given job ID.
    /// @param jobId The job ID to check.
    function supportsJob(uint8 jobId) external pure returns (bool) {
        return jobId <= JOB_WORKFLOW_CANCEL;
    }

    /// @notice Returns the total number of on-chain jobs this blueprint supports.
    function jobCount() external pure returns (uint256) {
        return 5;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR REGISTRATION
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Registers an operator. In cloud mode, sets capacity from registrationInputs.
    /// @param operator The operator address being registered.
    /// @param registrationInputs ABI-encoded uint32 capacity (0 = use default).
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
        // Instance mode: no-op (operators self-report lifecycle via reportProvisioned/reportDeprovisioned)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR UNREGISTRATION & DEPARTURE
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Prevents operators from unregistering while they hold active resources.
    /// @param operator The operator address being unregistered.
    function onUnregister(
        address operator
    ) external virtual override onlyFromTangle {
        if (operatorActiveSandboxes[operator] != 0) revert CannotLeaveWithActiveResources();
        // Note: We cannot iterate all services here since the base interface
        // only provides the operator address. Instance-mode provisions are
        // checked via onOperatorLeft (per-service) and canLeave.
    }

    /// @notice Prevents operators from leaving a service while they have active provisions.
    /// @param serviceId The service the operator is leaving.
    /// @param operator The departing operator address.
    function onOperatorLeft(
        uint64 serviceId,
        address operator
    ) external virtual override onlyFromTangle {
        if (operatorActiveSandboxes[operator] != 0) revert CannotLeaveWithActiveResources();
        if (operatorProvisioned[serviceId][operator]) revert CannotLeaveWithActiveResources();
    }

    /// @notice Pre-check: denies departure if operator has active provisions for this service.
    /// @param serviceId The service to check against.
    /// @param operator The operator requesting to leave.
    function canLeave(
        uint64 serviceId,
        address operator
    ) external view virtual override returns (bool) {
        if (operatorActiveSandboxes[operator] > 0) return false;
        if (operatorProvisioned[serviceId][operator]) return false;
        return true;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE REQUEST VALIDATION
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Validates a service request. In instance mode, stores pending config.
    ///         In cloud mode, validates operator selection against restaking state.
    /// @param requestId Unique identifier for this service request.
    /// @param requester Address of the service requester.
    /// @param operators Operator set proposed for this service.
    /// @param requestInputs ABI-encoded config (instance mode) or SelectionRequest (cloud mode).
    function onRequest(
        uint64 requestId,
        address requester,
        address[] calldata operators,
        bytes calldata requestInputs,
        uint64 ttl,
        address paymentAsset,
        uint256 paymentAmount
    ) external payable override onlyFromTangle {
        ttl;
        paymentAsset;
        paymentAmount;

        if (operators.length == 0) revert ZeroOperatorsInRequest();

        if (instanceMode) {
            // Store sandbox config for retrieval in onServiceInitialized
            if (requestInputs.length > 0) {
                _pendingRequestConfig[requestId] = requestInputs;
            }
        } else {
            SelectionRequest memory selection = _decodeSelectionRequest(requestInputs);
            _validateOperatorSelection(operators, selection);
        }

        emit ServiceRequestValidated(requestId, requester, uint32(operators.length));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE LIFECYCLE HOOKS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Called when the service is initialized. In instance mode, stores the service
    ///         owner and moves pending config to persistent storage keyed by serviceId.
    function onServiceInitialized(
        uint64,              // blueprintId
        uint64 requestId,
        uint64 serviceId,
        address owner,
        address[] calldata,  // permittedCallers
        uint64               // ttl
    ) external override onlyFromTangle {
        if (instanceMode) {
            if (serviceOwner[serviceId] != address(0)) revert ServiceAlreadyInitialized(serviceId);
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
    /// @param serviceId The service being terminated.
    /// @param owner The service owner who initiated termination.
    function onServiceTermination(
        uint64 serviceId,
        address owner
    ) external override onlyFromTangle {
        emit ServiceTerminationReceived(serviceId, owner);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB CALL HOOK — OPERATOR ASSIGNMENT & ROUTING
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Pre-execution hook for job calls. Assigns operators (create) or routes
    ///         to the sandbox's operator (delete). Enforces mode restrictions.
    /// @param serviceId The service this job belongs to.
    /// @param job The job ID being called.
    /// @param jobCallId Unique identifier for this job call.
    /// @param inputs ABI-encoded job inputs.
    function onJobCall(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        bytes calldata inputs
    ) external payable override onlyFromTangle {
        if (job == JOB_SANDBOX_CREATE) {
            if (instanceMode) revert CloudModeOnly();
            address selected = _selectByCapacity(serviceId);
            _createAssignments[serviceId][jobCallId] = selected;
            emit OperatorAssigned(serviceId, jobCallId, selected);
        } else if (job == JOB_SANDBOX_DELETE) {
            if (instanceMode) revert CloudModeOnly();
            SandboxIdRequest memory request = abi.decode(inputs, (SandboxIdRequest));
            string memory sandboxId = request.sandbox_id;
            bytes32 sandboxHash = keccak256(bytes(sandboxId));
            address routed = sandboxOperator[sandboxHash];
            if (routed == address(0)) revert SandboxNotFound(sandboxHash);
            emit OperatorRouted(serviceId, jobCallId, routed);
        } else if (job == JOB_WORKFLOW_CREATE || job == JOB_WORKFLOW_TRIGGER || job == JOB_WORKFLOW_CANCEL) {
            // Supported in both cloud and instance modes.
        } else {
            revert UnknownJobId(job);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB RESULT HOOK — STATE UPDATES
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Post-execution hook for job results. Updates on-chain state based on
    ///         operator-submitted results (sandbox registry, workflows).
    /// @param serviceId The service this job belongs to.
    /// @param job The job ID that completed.
    /// @param jobCallId Unique identifier for this job call.
    /// @param operator The operator that executed the job.
    /// @param inputs ABI-encoded original job inputs.
    /// @param outputs ABI-encoded job outputs from the operator.
    function onJobResult(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        address operator,
        bytes calldata inputs,
        bytes calldata outputs
    ) external payable override onlyFromTangle {
        if (job == JOB_SANDBOX_CREATE) {
            if (instanceMode) revert CloudModeOnly();
            _handleCreateResult(serviceId, jobCallId, operator, outputs);
        } else if (job == JOB_SANDBOX_DELETE) {
            if (instanceMode) revert CloudModeOnly();
            _handleDeleteResult(operator, inputs);
        } else if (job == JOB_WORKFLOW_CREATE) {
            WorkflowCreateRequest memory request = abi.decode(inputs, (WorkflowCreateRequest));
            _upsertWorkflow(jobCallId, request);
        } else if (job == JOB_WORKFLOW_TRIGGER) {
            WorkflowControlRequest memory request = abi.decode(inputs, (WorkflowControlRequest));
            _markTriggered(request.workflow_id);
        } else if (job == JOB_WORKFLOW_CANCEL) {
            WorkflowControlRequest memory request = abi.decode(inputs, (WorkflowControlRequest));
            _cancelWorkflow(request.workflow_id);
        } else {
            revert UnknownJobId(job);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // DIRECT INSTANCE REPORTING (operator-signed tx path)
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Operator-direct lifecycle report for instance mode provision.
    /// @dev Auth is the tx signer (`msg.sender`) plus service membership check.
    ///      This path avoids submitJob caller permission coupling for startup reconciliation.
    function reportProvisioned(
        uint64 serviceId,
        string calldata sandboxId,
        string calldata sidecarUrl,
        uint32 sshPort,
        string calldata teeAttestationJson
    ) external {
        if (!instanceMode) revert InstanceModeOnly();
        _requireActiveServiceOperator(serviceId, msg.sender);

        bytes memory outputs = abi.encode(sandboxId, sidecarUrl, sshPort, teeAttestationJson);
        _handleProvisionResult(serviceId, msg.sender, outputs);
    }

    /// @notice Operator-direct lifecycle report for instance mode deprovision.
    /// @dev Auth is the tx signer (`msg.sender`) plus service membership check.
    function reportDeprovisioned(uint64 serviceId) external {
        if (!instanceMode) revert InstanceModeOnly();
        _requireActiveServiceOperator(serviceId, msg.sender);
        _handleDeprovisionResult(serviceId, msg.sender);
    }

    /// @notice Returns the number of operator results required to finalize a job (always 1).
    function getRequiredResultCount(uint64, uint8) external pure override returns (uint32) {
        return 1;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // ADMIN FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Sets the default max sandbox capacity for newly registered operators.
    /// @param capacity New default capacity value.
    function setDefaultMaxCapacity(uint32 capacity) external onlyBlueprintOwner {
        defaultMaxCapacity = capacity;
    }

    /// @notice Overrides the max sandbox capacity for a specific operator.
    /// @param operator The operator whose capacity to set.
    /// @param capacity New capacity value.
    function setOperatorCapacity(address operator, uint32 capacity) external onlyBlueprintOwner {
        operatorMaxCapacity[operator] = capacity;
    }

    /// @notice Toggles instance mode. Cannot be changed while sandboxes or provisions exist.
    /// @param _mode True for instance mode, false for cloud mode.
    function setInstanceMode(bool _mode) external onlyBlueprintOwner {
        if (totalActiveSandboxes != 0 || totalProvisionedOperators != 0) revert CannotChangeWithActiveResources();
        instanceMode = _mode;
    }

    /// @notice Toggles TEE attestation requirement. Cannot be changed while resources exist.
    /// @param _required True to require TEE attestation on provision.
    function setTeeRequired(bool _required) external onlyBlueprintOwner {
        if (totalActiveSandboxes != 0 || totalProvisionedOperators != 0) revert CannotChangeWithActiveResources();
        teeRequired = _required;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS — CLOUD MODE
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Returns an operator's active sandbox count and max capacity.
    /// @param operator The operator address to query.
    function getOperatorLoad(address operator) external view returns (uint32 active, uint32 max) {
        return (operatorActiveSandboxes[operator], operatorMaxCapacity[operator]);
    }

    /// @notice Returns the operator assigned to a sandbox.
    /// @param sandboxId The sandbox identifier string.
    function getSandboxOperator(string calldata sandboxId) external view returns (address) {
        return sandboxOperator[keccak256(bytes(sandboxId))];
    }

    /// @notice Returns true if the given sandbox is currently active.
    /// @param sandboxId The sandbox identifier string.
    function isSandboxActive(string calldata sandboxId) external view returns (bool) {
        return sandboxActive[keccak256(bytes(sandboxId))];
    }

    /// @notice Returns the total remaining sandbox capacity across all eligible operators.
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

    /// @notice Returns aggregate stats: total active sandboxes and total operator capacity.
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

    /// @notice Returns true if at least one operator is provisioned for this service.
    /// @param serviceId The service to check.
    function isProvisioned(uint64 serviceId) external view returns (bool) {
        return instanceOperatorCount[serviceId] > 0;
    }

    /// @notice Returns true if a specific operator is provisioned for this service.
    /// @param serviceId The service to check.
    /// @param operator The operator address to check.
    function isOperatorProvisioned(uint64 serviceId, address operator) external view returns (bool) {
        return operatorProvisioned[serviceId][operator];
    }

    /// @notice Returns the number of provisioned operators for a service.
    /// @param serviceId The service to query.
    function getOperatorCount(uint64 serviceId) external view returns (uint32) {
        return instanceOperatorCount[serviceId];
    }

    /// @notice Returns the TEE attestation hash for an operator on a service.
    /// @param serviceId The service to query.
    /// @param operator The operator address.
    function getAttestationHash(uint64 serviceId, address operator) external view returns (bytes32) {
        return operatorAttestationHash[serviceId][operator];
    }

    /// @notice Returns all provisioned operators and their sidecar URLs for a service.
    /// @param serviceId The service to query.
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

    /// @notice Returns the ABI-encoded sandbox config stored for a service.
    /// @param serviceId The service to query.
    function getServiceConfig(uint64 serviceId) external view returns (bytes memory) {
        return serviceConfig[serviceId];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW VIEWS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Returns the full config for a workflow.
    /// @param workflowId The workflow to query.
    function getWorkflow(uint64 workflowId) external view returns (WorkflowConfig memory) {
        return workflows[workflowId];
    }

    /// @notice Returns all workflow IDs, optionally filtered to active-only.
    /// @param activeOnly When true, only returns IDs of active workflows.
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

    /// @notice Computes per-job pricing rates from a base rate and built-in multipliers.
    /// @param baseRate The base rate to multiply against each job's price multiplier.
    function getDefaultJobRates(uint256 baseRate)
        external
        pure
        returns (uint8[] memory jobIndexes, uint256[] memory rates)
    {
        jobIndexes = new uint8[](5);
        rates = new uint256[](5);

        jobIndexes[0] = JOB_SANDBOX_CREATE;    rates[0] = baseRate * PRICE_MULT_SANDBOX_CREATE;
        jobIndexes[1] = JOB_SANDBOX_DELETE;    rates[1] = baseRate * PRICE_MULT_SANDBOX_DELETE;
        jobIndexes[2] = JOB_WORKFLOW_CREATE;   rates[2] = baseRate * PRICE_MULT_WORKFLOW_CREATE;
        jobIndexes[3] = JOB_WORKFLOW_TRIGGER;  rates[3] = baseRate * PRICE_MULT_WORKFLOW_TRIGGER;
        jobIndexes[4] = JOB_WORKFLOW_CANCEL;   rates[4] = baseRate * PRICE_MULT_WORKFLOW_CANCEL;
    }

    /// @notice Returns the price multiplier for a given job ID (0 if unknown).
    /// @param jobId The job ID to look up.
    function getJobPriceMultiplier(uint8 jobId) external pure returns (uint256) {
        if (jobId == JOB_SANDBOX_CREATE)    return PRICE_MULT_SANDBOX_CREATE;
        if (jobId == JOB_SANDBOX_DELETE)    return PRICE_MULT_SANDBOX_DELETE;
        if (jobId == JOB_WORKFLOW_CREATE)   return PRICE_MULT_WORKFLOW_CREATE;
        if (jobId == JOB_WORKFLOW_TRIGGER)  return PRICE_MULT_WORKFLOW_TRIGGER;
        if (jobId == JOB_WORKFLOW_CANCEL)   return PRICE_MULT_WORKFLOW_CANCEL;
        return 0;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: CAPACITY-WEIGHTED OPERATOR SELECTION (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Selects an operator weighted by remaining capacity using pseudo-random selection.
    /// @param serviceId The service ID used as part of the random seed.
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

        // NOTE: block.prevrandao is proposer-influenceable. This is an accepted
        // trade-off for operator selection: operators are trusted service providers,
        // not adversarial bidders. A commit-reveal scheme would add complexity
        // without meaningful security benefit in this threat model.
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

    /// @notice Processes a sandbox create result: validates the job was assigned, registers
    ///         the sandbox, and increments capacity counters.
    /// @param serviceId The service the sandbox belongs to.
    /// @param jobCallId The job call ID used to look up the pre-assigned operator.
    /// @param operator The operator that submitted the result.
    /// @param outputs ABI-encoded (sandboxId, jsonMetadata).
    function _handleCreateResult(
        uint64 serviceId,
        uint64 jobCallId,
        address operator,
        bytes calldata outputs
    ) internal {
        address assigned = _createAssignments[serviceId][jobCallId];
        if (assigned == address(0) || assigned != operator) revert OperatorMismatch(assigned, operator);

        SandboxCreateOutput memory result = abi.decode(outputs, (SandboxCreateOutput));
        string memory sandboxId = result.sandboxId;
        if (bytes(sandboxId).length == 0) revert EmptySandboxId();
        if (bytes(sandboxId).length > 255) revert SandboxIdTooLong(bytes(sandboxId).length);
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        if (sandboxOperator[sandboxHash] != address(0)) revert SandboxAlreadyExists(sandboxHash);

        sandboxOperator[sandboxHash] = operator;
        sandboxActive[sandboxHash] = true;
        operatorActiveSandboxes[operator]++;
        totalActiveSandboxes++;

        delete _createAssignments[serviceId][jobCallId];

        emit SandboxCreated(sandboxHash, operator);
    }

    /// @notice Processes a sandbox delete result: verifies operator ownership, removes the
    ///         sandbox from the registry, and decrements capacity counters.
    /// @param operator The operator that executed the delete.
    /// @param inputs ABI-encoded sandbox ID string.
    function _handleDeleteResult(address operator, bytes calldata inputs) internal {
        SandboxIdRequest memory request = abi.decode(inputs, (SandboxIdRequest));
        string memory sandboxId = request.sandbox_id;
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        address expected = sandboxOperator[sandboxHash];
        if (expected == address(0)) revert SandboxNotFound(sandboxHash);
        if (expected != operator) revert OperatorMismatch(expected, operator);

        delete sandboxOperator[sandboxHash];
        sandboxActive[sandboxHash] = false;
        if (operatorActiveSandboxes[operator] > 0) {
            operatorActiveSandboxes[operator]--;
        }
        if (totalActiveSandboxes > 0) {
            totalActiveSandboxes--;
        }

        emit SandboxDeleted(sandboxHash, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: INSTANCE LIFECYCLE STATE TRANSITIONS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Processes a provision result: registers the operator, stores sidecar URL
    ///         and TEE attestation hash, increments counters.
    /// @param serviceId The service being provisioned.
    /// @param operator The operator that provisioned.
    /// @param outputs ABI-encoded (sandboxId, sidecarUrl, port, teeAttestationJson).
    function _handleProvisionResult(
        uint64 serviceId,
        address operator,
        bytes memory outputs
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
        totalProvisionedOperators++;

        operatorSidecarUrl[serviceId][operator] = sidecarUrl;
        if (_serviceOperators[serviceId].length >= MAX_OPERATORS_PER_SERVICE) revert MaxOperatorsReached(serviceId);
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

    /// @notice Processes a deprovision result: removes the operator from the service,
    ///         clears sidecar URL and attestation, decrements counters.
    /// @param serviceId The service being deprovisioned.
    /// @param operator The operator being removed.
    function _handleDeprovisionResult(
        uint64 serviceId,
        address operator
    ) internal {
        if (!operatorProvisioned[serviceId][operator]) revert NotProvisioned(serviceId, operator);

        operatorProvisioned[serviceId][operator] = false;
        if (instanceOperatorCount[serviceId] > 0) {
            instanceOperatorCount[serviceId]--;
        }
        if (totalProvisionedOperators > 0) {
            totalProvisionedOperators--;
        }

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

    /// @notice Creates or updates a workflow config in storage.
    /// @param workflowId The workflow ID (derived from jobCallId).
    /// @param request The workflow creation parameters.
    function _upsertWorkflow(uint64 workflowId, WorkflowCreateRequest memory request) internal {
        WorkflowConfig storage config = workflows[workflowId];
        if (workflow_index[workflowId] == 0) {
            if (workflow_ids.length >= MAX_WORKFLOWS) revert MaxWorkflowsReached(0);
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

    /// @notice Records a workflow trigger timestamp and emits WorkflowTriggered.
    /// @param workflowId The workflow that was triggered.
    function _markTriggered(uint64 workflowId) internal {
        if (workflow_index[workflowId] == 0) revert WorkflowNotFound(workflowId);
        WorkflowConfig storage config = workflows[workflowId];
        config.last_triggered_at = uint64(block.timestamp);
        config.updated_at = uint64(block.timestamp);
        emit WorkflowTriggered(workflowId, uint64(block.timestamp));
    }

    /// @notice Deactivates a workflow and emits WorkflowCanceled.
    /// @param workflowId The workflow to cancel.
    function _cancelWorkflow(uint64 workflowId) internal {
        if (workflow_index[workflowId] == 0) revert WorkflowNotFound(workflowId);
        WorkflowConfig storage config = workflows[workflowId];
        config.active = false;
        config.updated_at = uint64(block.timestamp);
        emit WorkflowCanceled(workflowId, uint64(block.timestamp));
    }

    /// @notice Validates that `operator` is currently active on `serviceId` in Tangle.
    function _requireActiveServiceOperator(uint64 serviceId, address operator) internal view {
        bool allowed = false;
        if (tangleCore != address(0)) {
            try ITangleServiceOperatorView(tangleCore).isServiceOperator(serviceId, operator) returns (bool active) {
                allowed = active;
            } catch { }
        }
        if (!allowed) revert OperatorNotInService(serviceId, operator);
    }
}
