// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./OperatorSelection.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";

/**
 * @title AgentSandboxBlueprint
 * @dev Multi-operator service manager for AI Agent Sandbox Blueprint.
 *      Handles capacity-weighted operator assignment for sandbox creation,
 *      on-chain routing of lifecycle operations, and workflow storage.
 */
contract AgentSandboxBlueprint is OperatorSelectionBase {
    // ═══════════════════════════════════════════════════════════════════════════
    // JOB IDS
    // ═══════════════════════════════════════════════════════════════════════════

    uint8 public constant JOB_SANDBOX_CREATE = 0;
    uint8 public constant JOB_SANDBOX_STOP = 1;
    uint8 public constant JOB_SANDBOX_RESUME = 2;
    uint8 public constant JOB_SANDBOX_DELETE = 3;
    uint8 public constant JOB_SANDBOX_SNAPSHOT = 4;

    uint8 public constant JOB_EXEC = 10;
    uint8 public constant JOB_PROMPT = 11;
    uint8 public constant JOB_TASK = 12;

    uint8 public constant JOB_BATCH_CREATE = 20;
    uint8 public constant JOB_BATCH_TASK = 21;
    uint8 public constant JOB_BATCH_EXEC = 22;
    uint8 public constant JOB_BATCH_COLLECT = 23;

    uint8 public constant JOB_WORKFLOW_CREATE = 30;
    uint8 public constant JOB_WORKFLOW_TRIGGER = 31;
    uint8 public constant JOB_WORKFLOW_CANCEL = 32;

    uint8 public constant JOB_SSH_PROVISION = 40;
    uint8 public constant JOB_SSH_REVOKE = 41;

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    string public constant BLUEPRINT_NAME = "ai-agent-sandbox-blueprint";
    string public constant BLUEPRINT_VERSION = "0.2.0";

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR CAPACITY STATE
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Maximum sandboxes an operator declared they can run.
    mapping(address => uint32) public operatorMaxCapacity;

    /// @notice Current active sandbox count per operator.
    mapping(address => uint32) public operatorActiveSandboxes;

    /// @notice Default capacity assigned when operator registers without specifying one.
    uint32 public defaultMaxCapacity = 100;

    /// @notice Global counter of active sandboxes across all operators.
    uint32 public totalActiveSandboxes;

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR ASSIGNMENT STATE
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Temporary assignment: serviceId → callId → assigned operator (for SANDBOX_CREATE).
    ///         Cleared after onJobResult processes the result.
    mapping(uint64 => mapping(uint64 => address)) internal _createAssignments;

    /// @notice Nonce for capacity-weighted selection entropy.
    uint256 internal _selectionNonce;

    // ═══════════════════════════════════════════════════════════════════════════
    // SANDBOX REGISTRY
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Routing: keccak256(sandboxId) → operator address.
    mapping(bytes32 => address) public sandboxOperator;

    /// @notice Whether a sandbox is currently active.
    mapping(bytes32 => bool) public sandboxActive;

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW STATE (preserved from v0.1)
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
    // EVENTS
    // ═══════════════════════════════════════════════════════════════════════════

    event OperatorAssigned(uint64 indexed serviceId, uint64 indexed callId, address indexed operator);
    event OperatorRouted(uint64 indexed serviceId, uint64 indexed callId, address indexed operator);
    event SandboxCreated(bytes32 indexed sandboxHash, address indexed operator);
    event SandboxDeleted(bytes32 indexed sandboxHash, address indexed operator);

    event WorkflowStored(uint64 indexed workflow_id, string trigger_type, string trigger_config);
    event WorkflowTriggered(uint64 indexed workflow_id, uint64 triggered_at);
    event WorkflowCanceled(uint64 indexed workflow_id, uint64 canceled_at);

    // ═══════════════════════════════════════════════════════════════════════════
    // ERRORS
    // ═══════════════════════════════════════════════════════════════════════════

    error NoAvailableCapacity();
    error OperatorMismatch(address expected, address actual);
    error SandboxNotFound(bytes32 sandboxHash);
    error SandboxAlreadyExists(bytes32 sandboxHash);

    // ═══════════════════════════════════════════════════════════════════════════
    // CONSTRUCTOR
    // ═══════════════════════════════════════════════════════════════════════════

    constructor(address restakingAddress) {
        if (restakingAddress != address(0)) {
            restaking = IMultiAssetDelegation(restakingAddress);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function jobIds() external pure returns (uint8[] memory ids) {
        ids = new uint8[](17);
        ids[0] = JOB_SANDBOX_CREATE;
        ids[1] = JOB_SANDBOX_STOP;
        ids[2] = JOB_SANDBOX_RESUME;
        ids[3] = JOB_SANDBOX_DELETE;
        ids[4] = JOB_SANDBOX_SNAPSHOT;
        ids[5] = JOB_EXEC;
        ids[6] = JOB_PROMPT;
        ids[7] = JOB_TASK;
        ids[8] = JOB_BATCH_CREATE;
        ids[9] = JOB_BATCH_TASK;
        ids[10] = JOB_BATCH_EXEC;
        ids[11] = JOB_BATCH_COLLECT;
        ids[12] = JOB_WORKFLOW_CREATE;
        ids[13] = JOB_WORKFLOW_TRIGGER;
        ids[14] = JOB_WORKFLOW_CANCEL;
        ids[15] = JOB_SSH_PROVISION;
        ids[16] = JOB_SSH_REVOKE;
    }

    function supportsJob(uint8 jobId) external pure returns (bool) {
        return jobId == JOB_SANDBOX_CREATE
            || jobId == JOB_SANDBOX_STOP
            || jobId == JOB_SANDBOX_RESUME
            || jobId == JOB_SANDBOX_DELETE
            || jobId == JOB_SANDBOX_SNAPSHOT
            || jobId == JOB_EXEC
            || jobId == JOB_PROMPT
            || jobId == JOB_TASK
            || jobId == JOB_BATCH_CREATE
            || jobId == JOB_BATCH_TASK
            || jobId == JOB_BATCH_EXEC
            || jobId == JOB_BATCH_COLLECT
            || jobId == JOB_WORKFLOW_CREATE
            || jobId == JOB_WORKFLOW_TRIGGER
            || jobId == JOB_WORKFLOW_CANCEL
            || jobId == JOB_SSH_PROVISION
            || jobId == JOB_SSH_REVOKE;
    }

    function jobCount() external pure returns (uint256) {
        return 17;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR REGISTRATION
    // ═══════════════════════════════════════════════════════════════════════════

    /**
     * @dev Operator registration hook. Decodes optional capacity from inputs.
     *      If inputs are empty or decode to 0, uses defaultMaxCapacity.
     */
    function onRegister(
        address operator,
        bytes calldata registrationInputs
    ) external payable override onlyFromTangle {
        uint32 capacity = defaultMaxCapacity;
        if (registrationInputs.length >= 32) {
            uint32 decoded = abi.decode(registrationInputs, (uint32));
            if (decoded > 0) {
                capacity = decoded;
            }
        }
        operatorMaxCapacity[operator] = capacity;
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
        requestId;
        requester;
        ttl;
        paymentAsset;
        paymentAmount;

        SelectionRequest memory selection = _decodeSelectionRequest(requestInputs);
        _validateOperatorSelection(operators, selection);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB CALL HOOK — OPERATOR ASSIGNMENT & ROUTING
    // ═══════════════════════════════════════════════════════════════════════════

    /**
     * @dev Called when a job is submitted. For sandbox lifecycle jobs,
     *      assigns or routes to the correct operator.
     */
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
        } else if (
            job == JOB_SANDBOX_STOP
            || job == JOB_SANDBOX_RESUME
            || job == JOB_SANDBOX_DELETE
            || job == JOB_SANDBOX_SNAPSHOT
        ) {
            string memory sandboxId = abi.decode(inputs, (string));
            bytes32 sandboxHash = keccak256(bytes(sandboxId));
            address routed = sandboxOperator[sandboxHash];
            if (routed == address(0)) revert SandboxNotFound(sandboxHash);
            emit OperatorRouted(serviceId, jobCallId, routed);
        }
        // Exec/prompt/task/batch/workflow/ssh: no on-chain routing needed.
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB RESULT HOOK — STATE UPDATES
    // ═══════════════════════════════════════════════════════════════════════════

    /**
     * @dev Called when an operator submits a job result. Validates operator
     *      assignment and updates sandbox registry / load counters.
     */
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
        } else if (job == JOB_SANDBOX_STOP || job == JOB_SANDBOX_RESUME || job == JOB_SANDBOX_SNAPSHOT) {
            _validateSandboxOperator(operator, inputs);
        } else if (job == JOB_WORKFLOW_CREATE) {
            WorkflowCreateRequest memory request = abi.decode(inputs, (WorkflowCreateRequest));
            _upsert_workflow(jobCallId, request);
        } else if (job == JOB_WORKFLOW_TRIGGER) {
            WorkflowControlRequest memory request = abi.decode(inputs, (WorkflowControlRequest));
            _mark_triggered(request.workflow_id);
        } else if (job == JOB_WORKFLOW_CANCEL) {
            WorkflowControlRequest memory request = abi.decode(inputs, (WorkflowControlRequest));
            _cancel_workflow(request.workflow_id);
        }
    }

    function getRequiredResultCount(uint64, uint8 job) external view override returns (uint32) {
        if (
            job == JOB_BATCH_CREATE
            || job == JOB_BATCH_TASK
            || job == JOB_BATCH_EXEC
        ) {
            return 0;
        }
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

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS
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
    // WORKFLOW VIEWS (preserved from v0.1)
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
    // INTERNAL: CAPACITY-WEIGHTED OPERATOR SELECTION
    // ═══════════════════════════════════════════════════════════════════════════

    /**
     * @dev Select an operator weighted by available capacity.
     *      Operators with more room get proportionally more assignments.
     *      Uses prevrandao + nonce for entropy (adequate for load balancing).
     */
    function _selectByCapacity(uint64 serviceId) internal returns (address) {
        if (address(restaking) == address(0)) revert RestakingNotSet();

        uint256 total = restaking.operatorCount();
        // Build arrays of eligible operators and their weights
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

        // Should not reach here, but return last candidate as safety.
        return candidates[count - 1];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: SANDBOX CREATE RESULT HANDLING
    // ═══════════════════════════════════════════════════════════════════════════

    function _handleCreateResult(
        uint64 serviceId,
        uint64 jobCallId,
        address operator,
        bytes calldata outputs
    ) internal {
        address assigned = _createAssignments[serviceId][jobCallId];
        if (assigned != operator) revert OperatorMismatch(assigned, operator);

        // Decode new output format: (string sandboxId, string json)
        (string memory sandboxId,) = abi.decode(outputs, (string, string));
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        if (sandboxOperator[sandboxHash] != address(0)) revert SandboxAlreadyExists(sandboxHash);

        sandboxOperator[sandboxHash] = operator;
        sandboxActive[sandboxHash] = true;
        operatorActiveSandboxes[operator]++;
        totalActiveSandboxes++;

        // Clean up temporary assignment
        delete _createAssignments[serviceId][jobCallId];

        emit SandboxCreated(sandboxHash, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: SANDBOX DELETE RESULT HANDLING
    // ═══════════════════════════════════════════════════════════════════════════

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
    // INTERNAL: SANDBOX OPERATOR VALIDATION (STOP/RESUME/SNAPSHOT)
    // ═══════════════════════════════════════════════════════════════════════════

    function _validateSandboxOperator(address operator, bytes calldata inputs) internal view {
        string memory sandboxId = abi.decode(inputs, (string));
        bytes32 sandboxHash = keccak256(bytes(sandboxId));
        address expected = sandboxOperator[sandboxHash];
        if (expected == address(0)) revert SandboxNotFound(sandboxHash);
        if (expected != operator) revert OperatorMismatch(expected, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: WORKFLOW STORAGE (preserved from v0.1)
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
