// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "tnt-core/BlueprintServiceManagerBase.sol";

/**
 * @title AgentSandboxBlueprint
 * @dev Service manager hooks for the AI Agent Sandbox Blueprint.
 */
contract AgentSandboxBlueprint is BlueprintServiceManagerBase {
    /// Job IDs (write-only sandbox + sidecar operations).
    uint8 public constant JOB_SANDBOX_CREATE = 0;
    uint8 public constant JOB_SANDBOX_DELETE = 3;
    uint8 public constant JOB_SANDBOX_STOP = 4;
    uint8 public constant JOB_SANDBOX_RESUME = 5;
    uint8 public constant JOB_SANDBOX_EXEC = 6;
    uint8 public constant JOB_SANDBOX_PROMPT = 7;

    /// Blueprint metadata helpers.
    string public constant BLUEPRINT_NAME = "ai-agent-sandbox-blueprint";
    string public constant BLUEPRINT_VERSION = "0.1.0";

    /// @notice Returns all supported job IDs for this blueprint.
    function jobIds() external pure returns (uint8[] memory ids) {
        ids = new uint8[](6);
        ids[0] = JOB_SANDBOX_CREATE;
        ids[1] = JOB_SANDBOX_DELETE;
        ids[2] = JOB_SANDBOX_STOP;
        ids[3] = JOB_SANDBOX_RESUME;
        ids[4] = JOB_SANDBOX_EXEC;
        ids[5] = JOB_SANDBOX_PROMPT;
    }

    /// @notice Returns true if the job ID is supported by this blueprint.
    function supportsJob(uint8 jobId) external pure returns (bool) {
        return jobId == JOB_SANDBOX_CREATE
            || jobId == JOB_SANDBOX_DELETE
            || jobId == JOB_SANDBOX_STOP
            || jobId == JOB_SANDBOX_RESUME
            || jobId == JOB_SANDBOX_EXEC
            || jobId == JOB_SANDBOX_PROMPT;
    }

    /// @notice Count of supported jobs.
    function jobCount() external pure returns (uint256) {
        return 6;
    }

    /**
     * @dev Hook for service operator registration. Called when a service operator
     * attempts to register with the blueprint.
     * @param operator The operator's details.
     * @param registrationInputs Inputs required for registration in bytes format.
     */
    function onRegister(
        ServiceOperators.OperatorPreferences calldata operator,
        bytes calldata registrationInputs
    )
    external
    payable
    virtual
    override
    onlyFromMaster
    {
        operator;
        registrationInputs;
    }

    /**
     *  @dev Hook for service instance requests. Called when a user requests a service
     *  instance from the blueprint but this does not mean the service is initiated yet.
     *  To get notified when the service is initiated, implement the `onServiceInitialized` hook.
     *
     *  @param params The parameters for the service request.
     */
    function onRequest(ServiceOperators.RequestParams calldata params) external payable virtual override onlyFromMaster
    {
        params;
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
        ServiceOperators.OperatorPreferences calldata operator,
        bytes calldata inputs,
        bytes calldata outputs
    )
    external
    payable
    virtual
    override
    onlyFromMaster
    {
        serviceId;
        job;
        jobCallId;
        operator;
        inputs;
        outputs;
    }
}
