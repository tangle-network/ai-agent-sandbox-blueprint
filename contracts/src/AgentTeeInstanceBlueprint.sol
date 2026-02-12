// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/BlueprintServiceManagerBase.sol";

/**
 * @title AgentTeeInstanceBlueprint
 * @dev TEE-backed service manager for AI Agent Instance Blueprint.
 *      Identical to AgentInstanceBlueprint except:
 *        - Higher pricing multipliers (operator fronts CVM fees).
 *        - Attestation enforcement: provision must include a non-empty
 *          teeAttestationJson or the result is rejected.
 */
contract AgentTeeInstanceBlueprint is BlueprintServiceManagerBase {
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

    string public constant BLUEPRINT_NAME = "ai-agent-tee-instance-blueprint";
    string public constant BLUEPRINT_VERSION = "0.1.0";

    // ═══════════════════════════════════════════════════════════════════════════
    // PER-JOB PRICING MULTIPLIERS (TEE — higher than base instance)
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // TEE multipliers reflect the higher cost of CVM execution:
    //   - PROVISION: 500x (CVM creation, vs 50x for Docker)
    //   - DEPROVISION: 5x (CVM teardown, vs 1x)
    //   - EXEC: 2x (encrypted memory overhead, vs 1x)
    //   - PROMPT: 30x (vs 20x)
    //   - TASK: 350x (vs 250x)
    //   - SNAPSHOT: 10x (vs 5x)
    //   - SSH_PROVISION: 3x (vs 2x)
    //   - SSH_REVOKE: 1x (same)

    uint256 public constant PRICE_MULT_EXEC = 2;
    uint256 public constant PRICE_MULT_SSH_REVOKE = 1;
    uint256 public constant PRICE_MULT_DEPROVISION = 5;
    uint256 public constant PRICE_MULT_SSH_PROVISION = 3;
    uint256 public constant PRICE_MULT_SNAPSHOT = 10;
    uint256 public constant PRICE_MULT_PROMPT = 30;
    uint256 public constant PRICE_MULT_PROVISION = 500;
    uint256 public constant PRICE_MULT_TASK = 350;

    // ═══════════════════════════════════════════════════════════════════════════
    // INSTANCE STATE
    // ═══════════════════════════════════════════════════════════════════════════

    mapping(uint64 => uint32) public instanceOperatorCount;
    mapping(uint64 => mapping(address => bool)) public operatorProvisioned;
    mapping(uint64 => mapping(address => bytes32)) public operatorAttestationHash;
    mapping(uint64 => address[]) internal _serviceOperators;
    mapping(uint64 => mapping(address => uint256)) internal _operatorIndex;
    mapping(uint64 => mapping(address => string)) public operatorSidecarUrl;
    mapping(uint64 => mapping(uint64 => mapping(address => bytes32))) public jobResultHash;

    // ═══════════════════════════════════════════════════════════════════════════
    // EVENTS
    // ═══════════════════════════════════════════════════════════════════════════

    event OperatorProvisioned(uint64 indexed serviceId, address indexed operator, string sandboxId, string sidecarUrl);
    event OperatorDeprovisioned(uint64 indexed serviceId, address indexed operator);
    event TeeAttestationStored(uint64 indexed serviceId, address indexed operator, bytes32 attestationHash);
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
    error MissingTeeAttestation(uint64 serviceId, address operator);

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

        require(operators.length >= 1, "At least 1 operator required");
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
            if (!operatorProvisioned[serviceId][operator]) {
                revert NotProvisioned(serviceId, operator);
            }

            if (job == JOB_PROMPT || job == JOB_TASK) {
                bytes32 resultHash = keccak256(outputs);
                jobResultHash[serviceId][jobCallId][operator] = resultHash;
                emit OperatorResultSubmitted(serviceId, jobCallId, operator, job, resultHash);
            }
        }
    }

    function getRequiredResultCount(uint64 serviceId, uint8 job) external view override returns (uint32) {
        if (job == JOB_PROMPT || job == JOB_TASK) {
            uint32 count = instanceOperatorCount[serviceId];
            return count > 0 ? count : 1;
        }
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
     *         scaled from a single base rate. TEE rates are higher than base.
     * @param baseRate The cost of the cheapest job (SSH_REVOKE) in native token wei.
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
     *      TEE variant: REQUIRES non-empty teeAttestationJson.
     */
    function _handleProvisionResult(
        uint64 serviceId,
        address operator,
        bytes calldata outputs
    ) internal {
        if (operatorProvisioned[serviceId][operator]) revert AlreadyProvisioned(serviceId, operator);

        (string memory sandboxId, string memory sidecarUrl,, string memory teeAttestationJson) =
            abi.decode(outputs, (string, string, uint32, string));

        // TEE attestation is mandatory for this blueprint.
        if (bytes(teeAttestationJson).length == 0) {
            revert MissingTeeAttestation(serviceId, operator);
        }

        operatorProvisioned[serviceId][operator] = true;
        instanceOperatorCount[serviceId]++;

        operatorSidecarUrl[serviceId][operator] = sidecarUrl;
        _serviceOperators[serviceId].push(operator);
        _operatorIndex[serviceId][operator] = _serviceOperators[serviceId].length;

        bytes32 attestationHash = keccak256(bytes(teeAttestationJson));
        operatorAttestationHash[serviceId][operator] = attestationHash;
        emit TeeAttestationStored(serviceId, operator, attestationHash);

        emit OperatorProvisioned(serviceId, operator, sandboxId, sidecarUrl);
    }

    function _handleDeprovisionResult(
        uint64 serviceId,
        address operator
    ) internal {
        if (!operatorProvisioned[serviceId][operator]) revert NotProvisioned(serviceId, operator);

        operatorProvisioned[serviceId][operator] = false;
        instanceOperatorCount[serviceId]--;

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
