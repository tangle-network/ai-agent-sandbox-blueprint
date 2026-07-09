// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./OperatorSelection.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";
import "./libraries/SandboxStorage.sol";
import "./libraries/SandboxTypes.sol";
import "./libraries/SandboxLogic.sol";

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
 *
 *      Heavy internal handlers (capacity selection, sandbox create/delete, instance
 *      provision/deprovision, workflow CRUD) are extracted to `SandboxLogic` so the
 *      blueprint's deployed runtime sits well under the EIP-170 24,576 B cap that
 *      Hyperliquid (and every other strict EVM L2) enforces. Mutable state lives at
 *      a single ERC-7201 slot (`SandboxStorage`) so the contract and every library
 *      see the same fields without storage-ref threading.
 */
contract AgentSandboxBlueprint is OperatorSelectionBase {
    using SandboxLogic for *;

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB IDS (5 total — state-changing only)
    // ═══════════════════════════════════════════════════════════════════════════

    uint8 public constant JOB_SANDBOX_CREATE = 0;
    uint8 public constant JOB_SANDBOX_DELETE = 1;
    uint8 public constant JOB_WORKFLOW_CREATE = 2;
    uint8 public constant JOB_WORKFLOW_TRIGGER = 3;
    uint8 public constant JOB_WORKFLOW_CANCEL = 4;
    uint8 public constant WORKFLOW_TARGET_SANDBOX = 0;
    uint8 public constant WORKFLOW_TARGET_INSTANCE = 1;

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    string public constant BLUEPRINT_NAME = "ai-agent-sandbox-blueprint";
    string public constant BLUEPRINT_VERSION = "0.4.0";

    // ═══════════════════════════════════════════════════════════════════════════
    // PER-JOB PRICING MULTIPLIERS
    // ═══════════════════════════════════════════════════════════════════════════

    uint256 public constant PRICE_MULT_SANDBOX_CREATE = 50;
    uint256 public constant PRICE_MULT_SANDBOX_DELETE = 1;
    uint256 public constant PRICE_MULT_WORKFLOW_CREATE = 2;
    uint256 public constant PRICE_MULT_WORKFLOW_TRIGGER = 5;
    uint256 public constant PRICE_MULT_WORKFLOW_CANCEL = 1;

    // ═══════════════════════════════════════════════════════════════════════════
    // ARRAY BOUNDS (storage-griefing prevention)
    // ═══════════════════════════════════════════════════════════════════════════

    uint256 public constant MAX_WORKFLOWS = 10000;
    uint32 public constant MAX_OPERATORS_PER_SERVICE = 1000;
    uint256 public constant MAX_SANDBOX_ID_LENGTH = 255;

    // ═══════════════════════════════════════════════════════════════════════════
    // EVENTS — re-declared at contract level so the deployed ABI surface is
    // identical to pre-refactor. Library functions emit these signatures
    // verbatim via `emit SandboxTypes.<Event>(...)`.
    // ═══════════════════════════════════════════════════════════════════════════

    event OperatorAssigned(uint64 indexed serviceId, uint64 indexed callId, address indexed operator);
    event OperatorRouted(uint64 indexed serviceId, uint64 indexed callId, address indexed operator);
    event SandboxCreated(bytes32 indexed sandboxHash, address indexed operator);
    event SandboxDeleted(bytes32 indexed sandboxHash, address indexed operator);
    event WorkflowStored(uint64 indexed workflow_id, string trigger_type, string trigger_config);
    event WorkflowTriggered(uint64 indexed workflow_id, uint64 triggered_at);
    event WorkflowCanceled(uint64 indexed workflow_id, uint64 canceled_at);

    event OperatorProvisioned(uint64 indexed serviceId, address indexed operator, string sandboxId, string sidecarUrl);
    event OperatorDeprovisioned(uint64 indexed serviceId, address indexed operator);
    event TeeAttestationStored(uint64 indexed serviceId, address indexed operator, bytes32 attestationHash);
    event ServiceTerminationReceived(uint64 indexed serviceId, address indexed owner);
    event ServiceConfigStored(uint64 indexed serviceId, uint64 indexed requestId);

    event ServiceRequestValidated(uint64 indexed requestId, address requester, uint32 operatorCount);

    // ═══════════════════════════════════════════════════════════════════════════
    // ERRORS — entry-point-only errors stay on the contract for ABI parity;
    // library errors live in `SandboxTypes`.
    // ═══════════════════════════════════════════════════════════════════════════

    error OperatorNotInService(uint64 serviceId, address operator);
    error CloudModeOnly();
    error InstanceModeOnly();
    error UnknownJobId(uint8 jobId);
    /// @notice Cached onJobCall inputs did not hash to the inputsHash 0.19 supplied at result time.
    error InputsHashMismatch(uint64 jobCallId, bytes32 got, bytes32 expected);
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
        SandboxStorage.Data storage $ = SandboxStorage.load();
        $.instanceMode = _instanceMode;
        $.teeRequired = _teeRequired;
        $.defaultMaxCapacity = 100;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PUBLIC STATE GETTERS (back-compat for previous public state variables)
    // ═══════════════════════════════════════════════════════════════════════════

    function instanceMode() external view returns (bool) {
        return SandboxStorage.load().instanceMode;
    }

    function teeRequired() external view returns (bool) {
        return SandboxStorage.load().teeRequired;
    }

    function operatorMaxCapacity(address op) external view returns (uint32) {
        return SandboxStorage.load().operatorMaxCapacity[op];
    }

    function operatorActiveSandboxes(address op) external view returns (uint32) {
        return SandboxStorage.load().operatorActiveSandboxes[op];
    }

    function defaultMaxCapacity() external view returns (uint32) {
        return SandboxStorage.load().defaultMaxCapacity;
    }

    function totalActiveSandboxes() external view returns (uint32) {
        return SandboxStorage.load().totalActiveSandboxes;
    }

    function sandboxOperator(bytes32 sandboxHash) external view returns (address) {
        return SandboxStorage.load().sandboxOperator[sandboxHash];
    }

    function sandboxActive(bytes32 sandboxHash) external view returns (bool) {
        return SandboxStorage.load().sandboxActive[sandboxHash];
    }

    function instanceOperatorCount(uint64 serviceId) external view returns (uint32) {
        return SandboxStorage.load().instanceOperatorCount[serviceId];
    }

    function operatorProvisioned(uint64 serviceId, address operator) external view returns (bool) {
        return SandboxStorage.load().operatorProvisioned[serviceId][operator];
    }

    function operatorAttestationHash(uint64 serviceId, address operator) external view returns (bytes32) {
        return SandboxStorage.load().operatorAttestationHash[serviceId][operator];
    }

    function operatorSidecarUrl(uint64 serviceId, address operator) external view returns (string memory) {
        return SandboxStorage.load().operatorSidecarUrl[serviceId][operator];
    }

    function totalProvisionedOperators() external view returns (uint256) {
        return SandboxStorage.load().totalProvisionedOperators;
    }

    function serviceConfig(uint64 serviceId) external view returns (bytes memory) {
        return SandboxStorage.load().serviceConfig[serviceId];
    }

    function serviceOwner(uint64 serviceId) external view returns (address) {
        return SandboxStorage.load().serviceOwner[serviceId];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Returns all supported job IDs for this deployment mode.
    /// @dev Instance mode only exposes workflow jobs (2..4).
    function jobIds() external view returns (uint8[] memory ids) {
        if (SandboxStorage.load().instanceMode) {
            ids = new uint8[](3);
            ids[0] = JOB_WORKFLOW_CREATE;
            ids[1] = JOB_WORKFLOW_TRIGGER;
            ids[2] = JOB_WORKFLOW_CANCEL;
            return ids;
        }

        ids = new uint8[](5);
        ids[0] = JOB_SANDBOX_CREATE;
        ids[1] = JOB_SANDBOX_DELETE;
        ids[2] = JOB_WORKFLOW_CREATE;
        ids[3] = JOB_WORKFLOW_TRIGGER;
        ids[4] = JOB_WORKFLOW_CANCEL;
    }

    /// @notice Returns true if this blueprint supports the given job ID.
    function supportsJob(uint8 jobId) external view returns (bool) {
        if (SandboxStorage.load().instanceMode) {
            return jobId >= JOB_WORKFLOW_CREATE && jobId <= JOB_WORKFLOW_CANCEL;
        }
        return jobId <= JOB_WORKFLOW_CANCEL;
    }

    /// @notice Returns the total number of on-chain jobs exposed.
    function jobCount() external view returns (uint256) {
        return SandboxStorage.load().instanceMode ? 3 : 5;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR REGISTRATION
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Registers an operator. In cloud mode, sets capacity from registrationInputs.
    function onRegister(address operator, bytes calldata registrationInputs) external payable override onlyFromTangle {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if (!$.instanceMode) {
            uint32 capacity = $.defaultMaxCapacity;
            if (registrationInputs.length >= 32) {
                uint32 decoded = abi.decode(registrationInputs, (uint32));
                if (decoded > 0) {
                    capacity = decoded;
                }
            }
            $.operatorMaxCapacity[operator] = capacity;
        }
        // Instance mode: no-op (operators self-report lifecycle via reportProvisioned/reportDeprovisioned)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR UNREGISTRATION & DEPARTURE
    // ═══════════════════════════════════════════════════════════════════════════

    function onUnregister(address operator) external virtual override onlyFromTangle {
        if (SandboxStorage.load().operatorActiveSandboxes[operator] != 0) revert CannotLeaveWithActiveResources();
    }

    function onOperatorLeft(uint64 serviceId, address operator) external virtual override onlyFromTangle {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.operatorActiveSandboxes[operator] != 0) revert CannotLeaveWithActiveResources();
        if ($.operatorProvisioned[serviceId][operator]) revert CannotLeaveWithActiveResources();
    }

    function canLeave(uint64 serviceId, address operator) external view virtual override returns (bool) {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.operatorActiveSandboxes[operator] > 0) return false;
        if ($.operatorProvisioned[serviceId][operator]) return false;
        return true;
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
        ttl;
        paymentAsset;
        paymentAmount;

        if (operators.length == 0) revert ZeroOperatorsInRequest();

        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.instanceMode) {
            if (requestInputs.length > 0) {
                $.pendingRequestConfig[requestId] = requestInputs;
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

    function onServiceInitialized(uint64, uint64 requestId, uint64 serviceId, address owner, address[] calldata, uint64)
        external
        override
        onlyFromTangle
    {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.instanceMode) {
            if ($.serviceOwner[serviceId] != address(0)) revert ServiceAlreadyInitialized(serviceId);
            $.serviceOwner[serviceId] = owner;
            bytes memory cfg = $.pendingRequestConfig[requestId];
            if (cfg.length > 0) {
                $.serviceConfig[serviceId] = cfg;
                delete $.pendingRequestConfig[requestId];
                emit ServiceConfigStored(serviceId, requestId);
            }
        }
    }

    function onServiceTermination(uint64 serviceId, address owner) external override onlyFromTangle {
        emit ServiceTerminationReceived(serviceId, owner);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB CALL HOOK — OPERATOR ASSIGNMENT & ROUTING
    // ═══════════════════════════════════════════════════════════════════════════

    function onJobCall(uint64 serviceId, uint8 job, uint64 jobCallId, bytes calldata inputs)
        external
        payable
        override
        onlyFromTangle
    {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if (job == JOB_SANDBOX_CREATE) {
            if ($.instanceMode) revert CloudModeOnly();
            address selected = SandboxLogic.selectByCapacity(serviceId, _eligibleOperators());
            $.createAssignments[serviceId][jobCallId] = selected;
            emit OperatorAssigned(serviceId, jobCallId, selected);
        } else if (job == JOB_SANDBOX_DELETE) {
            if ($.instanceMode) revert CloudModeOnly();
            SandboxTypes.SandboxIdRequest memory request = abi.decode(inputs, (SandboxTypes.SandboxIdRequest));
            string memory sandboxId = request.sandbox_id;
            bytes32 sandboxHash = keccak256(bytes(sandboxId));
            address routed = $.sandboxOperator[sandboxHash];
            if (routed == address(0)) revert SandboxTypes.SandboxNotFound(sandboxHash);
            // tnt-core 0.19: onJobResult only receives inputsHash, so cache the
            // raw request here for handleDeleteResult to consume at result time.
            $.jobCallInputs[serviceId][jobCallId] = inputs;
            emit OperatorRouted(serviceId, jobCallId, routed);
        } else if (job == JOB_WORKFLOW_CREATE || job == JOB_WORKFLOW_TRIGGER || job == JOB_WORKFLOW_CANCEL) {
            // Supported in both cloud and instance modes. The workflow result
            // handlers decode the original inputs (config / workflowId), which
            // 0.19's onJobResult no longer forwards — cache them at call time.
            $.jobCallInputs[serviceId][jobCallId] = inputs;
        } else {
            revert UnknownJobId(job);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB RESULT HOOK — STATE UPDATES
    // ═══════════════════════════════════════════════════════════════════════════

    function onJobResult(
        uint64 serviceId,
        uint8 job,
        uint64 jobCallId,
        address operator,
        bytes32 inputsHash,
        bytes calldata outputs
    ) external payable override onlyFromTangle {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if (job == JOB_SANDBOX_CREATE) {
            // CREATE derives all state from `outputs` (the sandbox id the
            // operator returned); no cached inputs needed.
            if ($.instanceMode) revert CloudModeOnly();
            SandboxLogic.handleCreateResult(serviceId, jobCallId, operator, outputs);
        } else if (job == JOB_SANDBOX_DELETE) {
            if ($.instanceMode) revert CloudModeOnly();
            bytes memory inputs = _consumeJobCallInputs(serviceId, jobCallId, inputsHash);
            SandboxLogic.handleDeleteResult(operator, inputs);
        } else if (job == JOB_WORKFLOW_CREATE) {
            bytes memory inputs = _consumeJobCallInputs(serviceId, jobCallId, inputsHash);
            SandboxLogic.handleWorkflowCreateResult(serviceId, jobCallId, inputs);
        } else if (job == JOB_WORKFLOW_TRIGGER) {
            bytes memory inputs = _consumeJobCallInputs(serviceId, jobCallId, inputsHash);
            (uint64 workflowId) = abi.decode(inputs, (uint64));
            SandboxLogic.markTriggered(workflowId);
        } else if (job == JOB_WORKFLOW_CANCEL) {
            bytes memory inputs = _consumeJobCallInputs(serviceId, jobCallId, inputsHash);
            (uint64 workflowId) = abi.decode(inputs, (uint64));
            SandboxLogic.cancelWorkflow(workflowId);
        } else {
            revert UnknownJobId(job);
        }
    }

    /// @notice Reads the raw job inputs cached at `onJobCall`, binds them to the
    ///         0.19 result-time `inputsHash`, and clears the cache entry.
    /// @dev tnt-core 0.19 delivers only `inputsHash` to `onJobResult`; the raw
    ///      inputs are stashed by `onJobCall`. Asserting keccak256 equality
    ///      guarantees the consumed inputs are exactly those the caller hashed
    ///      over — a defense against any storage-cache desync or a forgotten
    ///      cache write (an empty entry hashes to keccak256("") and fails here).
    function _consumeJobCallInputs(uint64 serviceId, uint64 jobCallId, bytes32 inputsHash)
        internal
        returns (bytes memory inputs)
    {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        inputs = $.jobCallInputs[serviceId][jobCallId];
        if (keccak256(inputs) != inputsHash) revert InputsHashMismatch(jobCallId, keccak256(inputs), inputsHash);
        delete $.jobCallInputs[serviceId][jobCallId];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // DIRECT INSTANCE REPORTING (operator-signed tx path)
    // ═══════════════════════════════════════════════════════════════════════════

    function reportProvisioned(
        uint64 serviceId,
        string calldata sandboxId,
        string calldata sidecarUrl,
        uint32 sshPort,
        string calldata teeAttestationJson
    ) external {
        if (!SandboxStorage.load().instanceMode) revert InstanceModeOnly();
        _requireActiveServiceOperator(serviceId, msg.sender);

        bytes memory outputs = abi.encode(sandboxId, sidecarUrl, sshPort, teeAttestationJson);
        SandboxLogic.handleProvisionResult(serviceId, msg.sender, outputs);
    }

    function reportDeprovisioned(uint64 serviceId) external {
        if (!SandboxStorage.load().instanceMode) revert InstanceModeOnly();
        _requireActiveServiceOperator(serviceId, msg.sender);
        SandboxLogic.handleDeprovisionResult(serviceId, msg.sender);
    }

    function getRequiredResultCount(uint64, uint8) external pure override returns (uint32) {
        return 1;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // ADMIN FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════════════

    function setDefaultMaxCapacity(uint32 capacity) external onlyBlueprintOwner {
        SandboxStorage.load().defaultMaxCapacity = capacity;
    }

    function setOperatorCapacity(address operator, uint32 capacity) external onlyBlueprintOwner {
        SandboxStorage.load().operatorMaxCapacity[operator] = capacity;
    }

    function setInstanceMode(bool _mode) external onlyBlueprintOwner {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.totalActiveSandboxes != 0 || $.totalProvisionedOperators != 0) revert CannotChangeWithActiveResources();
        $.instanceMode = _mode;
    }

    function setTeeRequired(bool _required) external onlyBlueprintOwner {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.totalActiveSandboxes != 0 || $.totalProvisionedOperators != 0) revert CannotChangeWithActiveResources();
        $.teeRequired = _required;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════════════

    function getOperatorLoad(address operator) external view returns (uint32 active, uint32 max) {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        return ($.operatorActiveSandboxes[operator], $.operatorMaxCapacity[operator]);
    }

    function getSandboxOperator(string calldata sandboxId) external view returns (address) {
        return SandboxStorage.load().sandboxOperator[keccak256(bytes(sandboxId))];
    }

    function isSandboxActive(string calldata sandboxId) external view returns (bool) {
        return SandboxStorage.load().sandboxActive[keccak256(bytes(sandboxId))];
    }

    function getAvailableCapacity() external view returns (uint32 available) {
        if (address(restaking) == address(0)) return 0;
        return SandboxLogic.getAvailableCapacity(_eligibleOperators());
    }

    function getServiceStats() external view returns (uint32 totalSandboxes, uint32 totalCapacity) {
        totalSandboxes = SandboxStorage.load().totalActiveSandboxes;
        if (address(restaking) == address(0)) return (totalSandboxes, 0);
        (, totalCapacity) = SandboxLogic.getServiceStats(_eligibleOperators());
    }

    function isProvisioned(uint64 serviceId) external view returns (bool) {
        return SandboxStorage.load().instanceOperatorCount[serviceId] > 0;
    }

    function isOperatorProvisioned(uint64 serviceId, address operator) external view returns (bool) {
        return SandboxStorage.load().operatorProvisioned[serviceId][operator];
    }

    function getOperatorCount(uint64 serviceId) external view returns (uint32) {
        return SandboxStorage.load().instanceOperatorCount[serviceId];
    }

    function getAttestationHash(uint64 serviceId, address operator) external view returns (bytes32) {
        return SandboxStorage.load().operatorAttestationHash[serviceId][operator];
    }

    function getOperatorEndpoints(uint64 serviceId)
        external
        view
        returns (address[] memory operators, string[] memory sidecarUrls)
    {
        (operators, sidecarUrls) = SandboxLogic.getOperatorEndpoints(serviceId);
    }

    function getServiceConfig(uint64 serviceId) external view returns (bytes memory) {
        return SandboxStorage.load().serviceConfig[serviceId];
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW VIEWS
    // ═══════════════════════════════════════════════════════════════════════════

    function getWorkflow(uint64 workflowId) external view returns (SandboxTypes.WorkflowConfig memory) {
        return SandboxStorage.load().workflows[workflowId];
    }

    function getWorkflowIds(bool activeOnly) external view returns (uint64[] memory ids) {
        return SandboxLogic.getWorkflowIds(activeOnly);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PRICING HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    function getDefaultJobRates(uint256 baseRate)
        external
        view
        returns (uint8[] memory jobIndexes, uint256[] memory rates)
    {
        if (SandboxStorage.load().instanceMode) {
            jobIndexes = new uint8[](3);
            rates = new uint256[](3);

            jobIndexes[0] = JOB_WORKFLOW_CREATE;
            rates[0] = baseRate * PRICE_MULT_WORKFLOW_CREATE;
            jobIndexes[1] = JOB_WORKFLOW_TRIGGER;
            rates[1] = baseRate * PRICE_MULT_WORKFLOW_TRIGGER;
            jobIndexes[2] = JOB_WORKFLOW_CANCEL;
            rates[2] = baseRate * PRICE_MULT_WORKFLOW_CANCEL;
            return (jobIndexes, rates);
        }

        jobIndexes = new uint8[](5);
        rates = new uint256[](5);

        jobIndexes[0] = JOB_SANDBOX_CREATE;
        rates[0] = baseRate * PRICE_MULT_SANDBOX_CREATE;
        jobIndexes[1] = JOB_SANDBOX_DELETE;
        rates[1] = baseRate * PRICE_MULT_SANDBOX_DELETE;
        jobIndexes[2] = JOB_WORKFLOW_CREATE;
        rates[2] = baseRate * PRICE_MULT_WORKFLOW_CREATE;
        jobIndexes[3] = JOB_WORKFLOW_TRIGGER;
        rates[3] = baseRate * PRICE_MULT_WORKFLOW_TRIGGER;
        jobIndexes[4] = JOB_WORKFLOW_CANCEL;
        rates[4] = baseRate * PRICE_MULT_WORKFLOW_CANCEL;
    }

    function getJobPriceMultiplier(uint8 jobId) external view returns (uint256) {
        if (SandboxStorage.load().instanceMode && jobId < JOB_WORKFLOW_CREATE) return 0;
        if (jobId == JOB_SANDBOX_CREATE) return PRICE_MULT_SANDBOX_CREATE;
        if (jobId == JOB_SANDBOX_DELETE) return PRICE_MULT_SANDBOX_DELETE;
        if (jobId == JOB_WORKFLOW_CREATE) return PRICE_MULT_WORKFLOW_CREATE;
        if (jobId == JOB_WORKFLOW_TRIGGER) return PRICE_MULT_WORKFLOW_TRIGGER;
        if (jobId == JOB_WORKFLOW_CANCEL) return PRICE_MULT_WORKFLOW_CANCEL;
        return 0;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNALS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Validates that `operator` is currently active on `serviceId` in Tangle.
    function _requireActiveServiceOperator(uint64 serviceId, address operator) internal view {
        bool allowed = false;
        if (tangleCore != address(0)) {
            try ITangleServiceOperatorView(tangleCore).isServiceOperator(serviceId, operator) returns (bool active) {
                allowed = active;
            } catch {}
        }
        if (!allowed) revert OperatorNotInService(serviceId, operator);
    }
}
