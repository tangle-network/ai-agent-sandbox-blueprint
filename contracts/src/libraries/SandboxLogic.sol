// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/interfaces/IMultiAssetDelegation.sol";
import "./SandboxStorage.sol";
import "./SandboxTypes.sol";

/// @title SandboxLogic
/// @notice Heavy internal state transitions extracted from
///         `AgentSandboxBlueprint`. Every function is `external` so the
///         Solidity compiler emits the bytecode at the library address and
///         routes calls via DELEGATECALL — the blueprint keeps only thin
///         entry-point glue and lands well under the EIP-170 24,576 B cap.
///
///         State access goes through `SandboxStorage.load()`. DELEGATECALL
///         semantics mean writes hit the calling blueprint's storage.
library SandboxLogic {
    // ═══════════════════════════════════════════════════════════════════════════
    // CAPACITY-WEIGHTED OPERATOR SELECTION (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Selects an operator weighted by remaining capacity using
    ///         pseudo-random selection. Caller is expected to filter via
    ///         the inherited `_isEligibleOperator` invariant by passing
    ///         the eligible operator list in `eligibleOps`.
    /// @dev    `block.prevrandao` is proposer-influenceable. Accepted
    ///         trade-off for operator selection: operators are trusted
    ///         service providers, not adversarial bidders. A commit-reveal
    ///         scheme would add complexity without meaningful security
    ///         benefit in this threat model.
    function selectByCapacity(uint64 serviceId, address[] memory eligibleOps) external returns (address) {
        SandboxStorage.Data storage $ = SandboxStorage.load();

        uint256 total = eligibleOps.length;
        address[] memory candidates = new address[](total);
        uint32[] memory weights = new uint32[](total);
        uint32 totalWeight = 0;
        uint256 count = 0;

        for (uint256 i = 0; i < total; i++) {
            address op = eligibleOps[i];
            uint32 max = $.operatorMaxCapacity[op];
            uint32 active = $.operatorActiveSandboxes[op];
            if (max <= active) continue;
            uint32 weight = max - active;
            candidates[count] = op;
            weights[count] = weight;
            totalWeight += weight;
            count++;
        }

        if (count == 0 || totalWeight == 0) revert SandboxTypes.NoAvailableCapacity();

        uint256 rand = uint256(keccak256(abi.encode(block.prevrandao, serviceId, $.selectionNonce)));
        $.selectionNonce++;
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
    // SANDBOX CREATE / DELETE RESULTS (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function handleCreateResult(uint64 serviceId, uint64 jobCallId, address operator, bytes calldata outputs) external {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        address assigned = $.createAssignments[serviceId][jobCallId];
        if (assigned == address(0) || assigned != operator) {
            revert SandboxTypes.OperatorMismatch(assigned, operator);
        }

        SandboxTypes.SandboxCreateOutput memory result = abi.decode(outputs, (SandboxTypes.SandboxCreateOutput));
        string memory sandboxId = result.sandboxId;
        if (bytes(sandboxId).length == 0) revert SandboxTypes.EmptySandboxId();
        if (bytes(sandboxId).length > SandboxTypes.MAX_SANDBOX_ID_LENGTH) {
            revert SandboxTypes.SandboxIdTooLong(bytes(sandboxId).length);
        }
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        if ($.sandboxOperator[sandboxHash] != address(0)) revert SandboxTypes.SandboxAlreadyExists(sandboxHash);

        $.sandboxOperator[sandboxHash] = operator;
        $.sandboxActive[sandboxHash] = true;
        $.operatorActiveSandboxes[operator]++;
        $.totalActiveSandboxes++;

        delete $.createAssignments[serviceId][jobCallId];

        emit SandboxTypes.SandboxCreated(sandboxHash, operator);
    }

    function handleDeleteResult(address operator, bytes calldata inputs) external {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        SandboxTypes.SandboxIdRequest memory request = abi.decode(inputs, (SandboxTypes.SandboxIdRequest));
        string memory sandboxId = request.sandbox_id;
        bytes32 sandboxHash = keccak256(bytes(sandboxId));

        address expected = $.sandboxOperator[sandboxHash];
        if (expected == address(0)) revert SandboxTypes.SandboxNotFound(sandboxHash);
        if (expected != operator) revert SandboxTypes.OperatorMismatch(expected, operator);

        delete $.sandboxOperator[sandboxHash];
        $.sandboxActive[sandboxHash] = false;
        if ($.operatorActiveSandboxes[operator] > 0) {
            $.operatorActiveSandboxes[operator]--;
        }
        if ($.totalActiveSandboxes > 0) {
            $.totalActiveSandboxes--;
        }

        emit SandboxTypes.SandboxDeleted(sandboxHash, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INSTANCE LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    function handleProvisionResult(uint64 serviceId, address operator, bytes memory outputs) external {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.operatorProvisioned[serviceId][operator]) {
            revert SandboxTypes.AlreadyProvisioned(serviceId, operator);
        }

        (string memory sandboxId, string memory sidecarUrl,, string memory teeAttestationJson) =
            abi.decode(outputs, (string, string, uint32, string));

        if ($.teeRequired) {
            if (bytes(teeAttestationJson).length == 0) {
                revert SandboxTypes.MissingTeeAttestation(serviceId, operator);
            }
        }

        $.operatorProvisioned[serviceId][operator] = true;
        $.instanceOperatorCount[serviceId]++;
        $.totalProvisionedOperators++;

        $.operatorSidecarUrl[serviceId][operator] = sidecarUrl;
        if ($.serviceOperators[serviceId].length >= SandboxTypes.MAX_OPERATORS_PER_SERVICE) {
            revert SandboxTypes.MaxOperatorsReached(serviceId);
        }
        $.serviceOperators[serviceId].push(operator);
        $.operatorIndex[serviceId][operator] = $.serviceOperators[serviceId].length;

        if (bytes(teeAttestationJson).length > 0) {
            bytes32 attestationHash = keccak256(bytes(teeAttestationJson));
            $.operatorAttestationHash[serviceId][operator] = attestationHash;
            emit SandboxTypes.TeeAttestationStored(serviceId, operator, attestationHash);
        }

        emit SandboxTypes.OperatorProvisioned(serviceId, operator, sandboxId, sidecarUrl);
    }

    function handleDeprovisionResult(uint64 serviceId, address operator) external {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if (!$.operatorProvisioned[serviceId][operator]) {
            revert SandboxTypes.NotProvisioned(serviceId, operator);
        }

        $.operatorProvisioned[serviceId][operator] = false;
        if ($.instanceOperatorCount[serviceId] > 0) {
            $.instanceOperatorCount[serviceId]--;
        }
        if ($.totalProvisionedOperators > 0) {
            $.totalProvisionedOperators--;
        }

        // Swap-and-pop to remove operator from the enumerable list.
        uint256 index = $.operatorIndex[serviceId][operator];
        if (index > 0) {
            uint256 lastIndex = $.serviceOperators[serviceId].length;
            if (index != lastIndex) {
                address lastOperator = $.serviceOperators[serviceId][lastIndex - 1];
                $.serviceOperators[serviceId][index - 1] = lastOperator;
                $.operatorIndex[serviceId][lastOperator] = index;
            }
            $.serviceOperators[serviceId].pop();
            delete $.operatorIndex[serviceId][operator];
        }

        delete $.operatorSidecarUrl[serviceId][operator];
        delete $.operatorAttestationHash[serviceId][operator];

        emit SandboxTypes.OperatorDeprovisioned(serviceId, operator);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW STORAGE (both modes)
    // ═══════════════════════════════════════════════════════════════════════════

    function handleWorkflowCreateResult(uint64 serviceId, uint64 workflowId, bytes calldata inputs) external {
        (
            string memory name,
            string memory workflowJson,
            string memory triggerType,
            string memory triggerConfig,
            string memory sandboxConfigJson,
            uint8 targetKind,
            string memory targetSandboxId,
            uint64 targetServiceId
        ) = abi.decode(inputs, (string, string, string, string, string, uint8, string, uint64));
        _upsertWorkflow(
            serviceId,
            workflowId,
            name,
            workflowJson,
            triggerType,
            triggerConfig,
            sandboxConfigJson,
            targetKind,
            targetSandboxId,
            targetServiceId
        );
    }

    function markTriggered(uint64 workflowId) external {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.workflowIndex[workflowId] == 0) revert SandboxTypes.WorkflowNotFound(workflowId);
        SandboxTypes.WorkflowConfig storage config = $.workflows[workflowId];
        config.last_triggered_at = uint64(block.timestamp);
        config.updated_at = uint64(block.timestamp);
        emit SandboxTypes.WorkflowTriggered(workflowId, uint64(block.timestamp));
    }

    function cancelWorkflow(uint64 workflowId) external {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.workflowIndex[workflowId] == 0) revert SandboxTypes.WorkflowNotFound(workflowId);
        SandboxTypes.WorkflowConfig storage config = $.workflows[workflowId];
        config.active = false;
        config.updated_at = uint64(block.timestamp);
        emit SandboxTypes.WorkflowCanceled(workflowId, uint64(block.timestamp));
    }

    function _upsertWorkflow(
        uint64 serviceId,
        uint64 workflowId,
        string memory name,
        string memory workflowJson,
        string memory triggerType,
        string memory triggerConfig,
        string memory sandboxConfigJson,
        uint8 targetKind,
        string memory targetSandboxId,
        uint64 targetServiceId
    ) internal {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if ($.instanceMode) {
            if (targetKind != SandboxTypes.WORKFLOW_TARGET_INSTANCE) {
                revert SandboxTypes.InvalidWorkflowTarget(targetKind);
            }
            if (bytes(targetSandboxId).length != 0) revert SandboxTypes.InvalidWorkflowTarget(targetKind);
        } else {
            if (targetKind != SandboxTypes.WORKFLOW_TARGET_SANDBOX) {
                revert SandboxTypes.InvalidWorkflowTarget(targetKind);
            }
            if (bytes(targetSandboxId).length == 0) revert SandboxTypes.EmptySandboxId();
            if (bytes(targetSandboxId).length > SandboxTypes.MAX_SANDBOX_ID_LENGTH) {
                revert SandboxTypes.SandboxIdTooLong(bytes(targetSandboxId).length);
            }
        }
        if (targetServiceId != 0 && targetServiceId != serviceId) {
            revert SandboxTypes.InvalidWorkflowTarget(targetKind);
        }

        SandboxTypes.WorkflowConfig storage config = $.workflows[workflowId];
        if ($.workflowIndex[workflowId] == 0) {
            if ($.workflowIds.length >= SandboxTypes.MAX_WORKFLOWS) revert SandboxTypes.MaxWorkflowsReached(0);
            $.workflowIds.push(workflowId);
            $.workflowIndex[workflowId] = $.workflowIds.length;
            config.created_at = uint64(block.timestamp);
        }

        config.name = name;
        config.workflow_json = workflowJson;
        config.trigger_type = triggerType;
        config.trigger_config = triggerConfig;
        config.sandbox_config_json = sandboxConfigJson;
        config.target_kind = targetKind;
        config.target_sandbox_id = targetSandboxId;
        config.target_service_id = serviceId;
        config.active = true;
        config.updated_at = uint64(block.timestamp);

        emit SandboxTypes.WorkflowStored(workflowId, triggerType, triggerConfig);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW HELPERS (heavy reads — kept here to keep the contract small)
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Sum of per-operator remaining capacity, restricted to the
    ///         eligible operators the caller has already filtered.
    function getAvailableCapacity(address[] memory eligibleOps) external view returns (uint32 available) {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        for (uint256 i = 0; i < eligibleOps.length; i++) {
            address op = eligibleOps[i];
            uint32 max = $.operatorMaxCapacity[op];
            uint32 active = $.operatorActiveSandboxes[op];
            if (max > active) {
                available += (max - active);
            }
        }
    }

    function getServiceStats(address[] memory eligibleOps)
        external
        view
        returns (uint32 totalSandboxes, uint32 totalCapacity)
    {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        totalSandboxes = $.totalActiveSandboxes;
        for (uint256 i = 0; i < eligibleOps.length; i++) {
            totalCapacity += $.operatorMaxCapacity[eligibleOps[i]];
        }
    }

    function getOperatorEndpoints(uint64 serviceId)
        external
        view
        returns (address[] memory operators, string[] memory sidecarUrls)
    {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        operators = $.serviceOperators[serviceId];
        sidecarUrls = new string[](operators.length);
        for (uint256 i = 0; i < operators.length; i++) {
            sidecarUrls[i] = $.operatorSidecarUrl[serviceId][operators[i]];
        }
    }

    function getWorkflowIds(bool activeOnly) external view returns (uint64[] memory ids) {
        SandboxStorage.Data storage $ = SandboxStorage.load();
        if (!activeOnly) {
            return $.workflowIds;
        }

        uint256 total = $.workflowIds.length;
        uint256 count = 0;
        for (uint256 i = 0; i < total; i++) {
            if ($.workflows[$.workflowIds[i]].active) {
                count++;
            }
        }

        ids = new uint64[](count);
        uint256 idx = 0;
        for (uint256 i = 0; i < total; i++) {
            uint64 workflowId = $.workflowIds[i];
            if ($.workflows[workflowId].active) {
                ids[idx] = workflowId;
                idx++;
            }
        }
    }
}
