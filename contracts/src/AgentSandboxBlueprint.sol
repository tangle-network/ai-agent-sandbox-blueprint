// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./OperatorSelection.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";

/**
 * @title AgentSandboxBlueprint
 * @dev Service manager hooks for the AI Agent Sandbox Blueprint.
 */
contract AgentSandboxBlueprint is OperatorSelectionBase {
    /// Job IDs (write-only sandbox + sidecar operations).
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

    /// Blueprint metadata helpers.
    string public constant BLUEPRINT_NAME = "ai-agent-sandbox-blueprint";
    string public constant BLUEPRINT_VERSION = "0.1.0";

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

    event WorkflowStored(uint64 indexed workflow_id, string trigger_type, string trigger_config);
    event WorkflowTriggered(uint64 indexed workflow_id, uint64 triggered_at);
    event WorkflowCanceled(uint64 indexed workflow_id, uint64 canceled_at);

    constructor(address restakingAddress) {
        if (restakingAddress != address(0)) {
            restaking = IMultiAssetDelegation(restakingAddress);
        }
    }

    /// @notice Returns all supported job IDs for this blueprint.
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

    /// @notice Returns true if the job ID is supported by this blueprint.
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

    /// @notice Count of supported jobs.
    function jobCount() external pure returns (uint256) {
        return 17;
    }

    /**
     * @dev Hook for service operator registration. Called when a service operator
     * attempts to register with the blueprint.
     * @param operator The operator's details.
     * @param registrationInputs Inputs required for registration in bytes format.
     */
    function onRegister(address, bytes calldata) external payable override onlyFromTangle {
        // Accept all registrations by default.
    }

    /**
     *  @dev Hook for service instance requests. Called when a user requests a service
     *  instance from the blueprint but this does not mean the service is initiated yet.
     *  To get notified when the service is initiated, implement the `onServiceInitialized` hook.
     *
     *  @param params The parameters for the service request.
     */
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

    /**
     * @dev Hook for handling job result. Called when operators send the result
     * of a job execution.
     * @param serviceId The ID of the service related to the job.
     * @param job The job identifier.
     * @param jobCallId The unique ID for the job call.
     * @param operator The operator sending the result in bytes format.
     * @param inputs Inputs used for the job execution in bytes format.
     * @param outputs Outputs resulting from the job execution in bytes format.
     */
    function onJobResult(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        address operator,
        bytes calldata inputs,
        bytes calldata outputs
    ) external payable override onlyFromTangle {
        serviceId;
        job;
        jobCallId;
        operator;
        inputs;
        outputs;

        if (job == JOB_WORKFLOW_CREATE) {
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
            job == JOB_BATCH_CREATE ||
            job == JOB_BATCH_TASK ||
            job == JOB_BATCH_EXEC
        ) {
            return 0; // Require results from all service operators.
        }
        return 1;
    }

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
