// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/BlueprintServiceManagerBase.sol";

/**
 * @title AgentInstanceBlueprint
 * @dev Subscription-based service manager for AI Agent Instance Blueprint.
 *      Each service instance is a replicated AI agent sandbox — every operator
 *      in the service independently provisions and runs a copy of the same
 *      sandbox configuration. Customers choose how many operators they want
 *      (1 for simple, N for redundancy/verification).
 *
 *      Multi-operator model:
 *        - Each operator provisions their own sandbox independently.
 *        - The contract stores each operator's sidecar URL on provision.
 *        - Customers can enumerate all operator endpoints via `getOperatorEndpoints()`.
 *        - For exec/ssh/snapshot: any single provisioned operator result suffices.
 *        - For prompt/task: ALL operators must respond; the contract stores per-operator
 *          result hashes so customers can compare/aggregate the N responses off-chain.
 *        - Customers get N sidecar URLs and can stream output from each independently.
 *      Supports TEE attestation storage for customer-verifiable confidential execution.
 */
contract AgentInstanceBlueprint is BlueprintServiceManagerBase {
    // ═══════════════════════════════════════════════════════════════════════════
    // JOB IDS
    // ═══════════════════════════════════════════════════════════════════════════

    uint8 public constant JOB_PROVISION = 0;
    uint8 public constant JOB_EXEC = 1;
    uint8 public constant JOB_PROMPT = 2;
    uint8 public constant JOB_TASK = 3;
    uint8 public constant JOB_SSH_PROVISION = 4;
    uint8 public constant JOB_SSH_REVOKE = 5;
    uint8 public constant JOB_SNAPSHOT = 6;
    uint8 public constant JOB_DEPROVISION = 7;

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    string public constant BLUEPRINT_NAME = "ai-agent-instance-blueprint";
    string public constant BLUEPRINT_VERSION = "0.3.0";

    // ═══════════════════════════════════════════════════════════════════════════
    // PER-JOB PRICING MULTIPLIERS
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Even though this is a subscription model, Tangle settles per-job. The
    // multipliers price each job relative to a base rate (cost of a single exec).
    //
    // Tier 1 (1x): Trivial ops — EXEC, SSH_REVOKE, DEPROVISION
    // Tier 2 (2x): Light state changes — SSH_PROVISION
    // Tier 3 (5x): I/O-heavy — SNAPSHOT
    // Tier 4 (20x): Single LLM call — PROMPT
    // Tier 5 (50x): Container lifecycle — PROVISION
    // Tier 6 (250x): Multi-turn agent — TASK

    uint256 public constant PRICE_MULT_EXEC = 1;
    uint256 public constant PRICE_MULT_SSH_REVOKE = 1;
    uint256 public constant PRICE_MULT_DEPROVISION = 1;
    uint256 public constant PRICE_MULT_SSH_PROVISION = 2;
    uint256 public constant PRICE_MULT_SNAPSHOT = 5;
    uint256 public constant PRICE_MULT_PROMPT = 20;
    uint256 public constant PRICE_MULT_PROVISION = 50;
    uint256 public constant PRICE_MULT_TASK = 250;

    // ═══════════════════════════════════════════════════════════════════════════
    // INSTANCE STATE
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Number of operators that have provisioned for a service instance.
    mapping(uint64 => uint32) public instanceOperatorCount;

    /// @notice Whether a specific operator has provisioned for a service instance.
    mapping(uint64 => mapping(address => bool)) public operatorProvisioned;

    /// @notice TEE attestation hash per operator per service instance.
    mapping(uint64 => mapping(address => bytes32)) public operatorAttestationHash;

    /// @notice Ordered list of provisioned operators per service instance (for enumeration).
    mapping(uint64 => address[]) internal _serviceOperators;

    /// @notice Index+1 of each operator in the _serviceOperators array (0 = not present).
    mapping(uint64 => mapping(address => uint256)) internal _operatorIndex;

    /// @notice Sidecar URL per operator per service instance (set during provision).
    mapping(uint64 => mapping(address => string)) public operatorSidecarUrl;

    /// @notice Per-operator result hash for a specific job call.
    ///         serviceId => jobCallId => operator => keccak256(outputs)
    mapping(uint64 => mapping(uint64 => mapping(address => bytes32))) public jobResultHash;

    // ═══════════════════════════════════════════════════════════════════════════
    // EVENTS
    // ═══════════════════════════════════════════════════════════════════════════

    event OperatorProvisioned(uint64 indexed serviceId, address indexed operator, string sandboxId, string sidecarUrl);
    event OperatorDeprovisioned(uint64 indexed serviceId, address indexed operator);
    event TeeAttestationStored(uint64 indexed serviceId, address indexed operator, bytes32 attestationHash);
    event ServiceTerminationReceived(uint64 indexed serviceId, address indexed owner);
    event OperatorResultSubmitted(
        uint64 indexed serviceId,
        uint64 indexed jobCallId,
        address indexed operator,
        uint8 job,
        bytes32 resultHash
    );

    // ═══════════════════════════════════════════════════════════════════════════
    // ERRORS
    // ═══════════════════════════════════════════════════════════════════════════

    error AlreadyProvisioned(uint64 serviceId, address operator);
    error NotProvisioned(uint64 serviceId, address operator);
    error NoOperatorsProvisioned(uint64 serviceId);

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function jobIds() external pure returns (uint8[] memory ids) {
        ids = new uint8[](8);
        ids[0] = JOB_PROVISION;
        ids[1] = JOB_EXEC;
        ids[2] = JOB_PROMPT;
        ids[3] = JOB_TASK;
        ids[4] = JOB_SSH_PROVISION;
        ids[5] = JOB_SSH_REVOKE;
        ids[6] = JOB_SNAPSHOT;
        ids[7] = JOB_DEPROVISION;
    }

    function supportsJob(uint8 jobId) external pure returns (bool) {
        return jobId <= JOB_DEPROVISION;
    }

    function jobCount() external pure returns (uint256) {
        return 8;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR REGISTRATION
    // ═══════════════════════════════════════════════════════════════════════════

    function onRegister(
        address operator,
        bytes calldata registrationInputs
    ) external payable override onlyFromTangle {
        operator;
        registrationInputs;
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
        requestInputs;
        ttl;
        paymentAsset;
        paymentAmount;

        // At least 1 operator required. Customer chooses how many replicas.
        require(operators.length >= 1, "At least 1 operator required");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE TERMINATION
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Called by Tangle when the service owner terminates the service.
    ///         Emits an event so off-chain operators can detect termination and deprovision.
    function onServiceTermination(
        uint64 serviceId,
        address owner
    ) external override onlyFromTangle {
        emit ServiceTerminationReceived(serviceId, owner);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB CALL HOOK
    // ═══════════════════════════════════════════════════════════════════════════

    function onJobCall(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        bytes calldata inputs
    ) external payable override onlyFromTangle {
        jobCallId;
        inputs;

        // Provision and deprovision are always allowed (per-operator).
        // All other jobs require at least one operator to be provisioned.
        if (job != JOB_PROVISION && job != JOB_DEPROVISION) {
            if (instanceOperatorCount[serviceId] == 0) revert NoOperatorsProvisioned(serviceId);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB RESULT HOOK
    // ═══════════════════════════════════════════════════════════════════════════

    function onJobResult(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        address operator,
        bytes calldata inputs,
        bytes calldata outputs
    ) external payable override onlyFromTangle {
        inputs;

        if (job == JOB_PROVISION) {
            _handleProvisionResult(serviceId, operator, outputs);
        } else if (job == JOB_DEPROVISION) {
            _handleDeprovisionResult(serviceId, operator);
        } else {
            // For all other jobs, the operator must be provisioned.
            if (!operatorProvisioned[serviceId][operator]) {
                revert NotProvisioned(serviceId, operator);
            }

            // For prompt/task jobs, store the result hash so customers can
            // compare outputs across all N operators.
            if (job == JOB_PROMPT || job == JOB_TASK) {
                bytes32 resultHash = keccak256(outputs);
                jobResultHash[serviceId][jobCallId][operator] = resultHash;
                emit OperatorResultSubmitted(serviceId, jobCallId, operator, job, resultHash);
            }
        }
    }

    function getRequiredResultCount(uint64 serviceId, uint8 job) external view override returns (uint32) {
        // For prompt/task: require ALL provisioned operators to respond.
        // This lets customers compare/aggregate N independent LLM outputs.
        if (job == JOB_PROMPT || job == JOB_TASK) {
            uint32 count = instanceOperatorCount[serviceId];
            return count > 0 ? count : 1;
        }
        // For all other jobs (exec, ssh, snapshot, provision, deprovision):
        // a single operator response suffices.
        return 1;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS
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

    /// @notice Returns all provisioned operators and their sidecar URLs for a service instance.
    ///         Customers use this to get N sidecar URLs for streaming output from each operator.
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

    /// @notice Returns the result hashes submitted by each operator for a specific job call.
    ///         Useful for prompt/task jobs where all operators respond independently.
    function getJobResultHashes(uint64 serviceId, uint64 jobCallId)
        external
        view
        returns (address[] memory operators, bytes32[] memory resultHashes)
    {
        operators = _serviceOperators[serviceId];
        resultHashes = new bytes32[](operators.length);
        for (uint256 i = 0; i < operators.length; i++) {
            resultHashes[i] = jobResultHash[serviceId][jobCallId][operators[i]];
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PRICING HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    /**
     * @notice Returns the recommended per-job rates for all 8 job types,
     *         scaled from a single base rate.
     * @param baseRate The cost of the cheapest job (EXEC) in native token wei.
     */
    function getDefaultJobRates(uint256 baseRate)
        external
        pure
        returns (uint8[] memory jobIndexes, uint256[] memory rates)
    {
        jobIndexes = new uint8[](8);
        rates = new uint256[](8);

        jobIndexes[0] = JOB_PROVISION;      rates[0] = baseRate * PRICE_MULT_PROVISION;
        jobIndexes[1] = JOB_EXEC;           rates[1] = baseRate * PRICE_MULT_EXEC;
        jobIndexes[2] = JOB_PROMPT;         rates[2] = baseRate * PRICE_MULT_PROMPT;
        jobIndexes[3] = JOB_TASK;           rates[3] = baseRate * PRICE_MULT_TASK;
        jobIndexes[4] = JOB_SSH_PROVISION;  rates[4] = baseRate * PRICE_MULT_SSH_PROVISION;
        jobIndexes[5] = JOB_SSH_REVOKE;     rates[5] = baseRate * PRICE_MULT_SSH_REVOKE;
        jobIndexes[6] = JOB_SNAPSHOT;       rates[6] = baseRate * PRICE_MULT_SNAPSHOT;
        jobIndexes[7] = JOB_DEPROVISION;    rates[7] = baseRate * PRICE_MULT_DEPROVISION;
    }

    function getJobPriceMultiplier(uint8 jobId) external pure returns (uint256) {
        if (jobId == JOB_PROVISION)     return PRICE_MULT_PROVISION;
        if (jobId == JOB_EXEC)          return PRICE_MULT_EXEC;
        if (jobId == JOB_PROMPT)        return PRICE_MULT_PROMPT;
        if (jobId == JOB_TASK)          return PRICE_MULT_TASK;
        if (jobId == JOB_SSH_PROVISION) return PRICE_MULT_SSH_PROVISION;
        if (jobId == JOB_SSH_REVOKE)    return PRICE_MULT_SSH_REVOKE;
        if (jobId == JOB_SNAPSHOT)      return PRICE_MULT_SNAPSHOT;
        if (jobId == JOB_DEPROVISION)   return PRICE_MULT_DEPROVISION;
        return 0;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL
    // ═══════════════════════════════════════════════════════════════════════════

    /**
     * @dev Process provision result for a single operator.
     *      Each operator in the service independently provisions their own sandbox.
     *      Output format: (string sandbox_id, string sidecar_url, uint32 ssh_port, string tee_attestation_json)
     */
    function _handleProvisionResult(
        uint64 serviceId,
        address operator,
        bytes calldata outputs
    ) internal {
        if (operatorProvisioned[serviceId][operator]) revert AlreadyProvisioned(serviceId, operator);

        (string memory sandboxId, string memory sidecarUrl,, string memory teeAttestationJson) =
            abi.decode(outputs, (string, string, uint32, string));

        operatorProvisioned[serviceId][operator] = true;
        instanceOperatorCount[serviceId]++;

        // Store sidecar URL and add to operator list for enumeration.
        operatorSidecarUrl[serviceId][operator] = sidecarUrl;
        _serviceOperators[serviceId].push(operator);
        _operatorIndex[serviceId][operator] = _serviceOperators[serviceId].length; // 1-indexed

        // Store TEE attestation hash if present.
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

        // Swap-and-pop to remove operator from enumerable list.
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
}
