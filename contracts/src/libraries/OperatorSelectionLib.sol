// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/interfaces/IMultiAssetDelegation.sol";

/// @title OperatorSelectionLib
/// @notice Operator-set validation + deterministic selection helpers moved
///         out of `OperatorSelectionBase` so the inheriting blueprint stays
///         under EIP-170. Every function is `external` — DELEGATECALL'd
///         from the inheriting contract, executes in its storage context.
///
///         Callers pass eligibility / blueprint context explicitly so the
///         library doesn't have to know the inheriting contract's storage
///         layout. Selection randomness uses caller-supplied seeds; no
///         on-chain entropy is generated here.
library OperatorSelectionLib {
    error InvalidOperatorCount(uint32 requested, uint32 minOperators, uint32 maxOperators);
    error NotEnoughEligibleOperators(uint32 requested, uint32 available);
    error InvalidOperatorSelection();

    /// @notice Validate that `operators` matches `expectedCount` and either
    ///         equals the deterministic selection for `seed` (when
    ///         `enforceDeterministic`) or passes per-operator eligibility +
    ///         uniqueness checks.
    function validateOperatorSelection(
        address[] calldata operators,
        address[] memory eligibleOps,
        uint32 minOperators_,
        uint32 maxOperators_,
        uint32 expectedCount,
        bytes32 seed,
        bool enforceDeterministic
    ) external pure {
        if (operators.length != expectedCount) {
            revert InvalidOperatorCount(uint32(operators.length), minOperators_, maxOperators_);
        }

        if (enforceDeterministic) {
            address[] memory expected = _selectOperators(eligibleOps, expectedCount, seed, minOperators_, maxOperators_);
            if (expected.length != operators.length) revert InvalidOperatorSelection();
            for (uint256 i = 0; i < expected.length; i++) {
                if (expected[i] != operators[i]) revert InvalidOperatorSelection();
            }
            return;
        }

        // Non-deterministic mode: each submitted operator must be in the
        // eligible set, and the submitted set must be unique.
        for (uint256 i = 0; i < operators.length; i++) {
            address operator = operators[i];
            bool ok = false;
            for (uint256 k = 0; k < eligibleOps.length; k++) {
                if (eligibleOps[k] == operator) {
                    ok = true;
                    break;
                }
            }
            if (!ok) revert InvalidOperatorSelection();
            for (uint256 j = i + 1; j < operators.length; j++) {
                if (operators[j] == operator) revert InvalidOperatorSelection();
            }
        }
    }

    /// @notice Deterministic selection of `operatorCount` operators from
    ///         `eligibleOps` using `seed`. Pure — no entropy from chain
    ///         state. Caller is expected to pass a stable per-request seed.
    function selectOperators(
        address[] memory eligibleOps,
        uint32 operatorCount,
        bytes32 seed,
        uint32 minOperators_,
        uint32 maxOperators_
    ) external pure returns (address[] memory) {
        return _selectOperators(eligibleOps, operatorCount, seed, minOperators_, maxOperators_);
    }

    /// @notice Filter the full operator set via `restaking` to those active
    ///         and registered against `blueprintId`.
    function eligibleOperators(IMultiAssetDelegation restaking, uint256 blueprintId_)
        external
        view
        returns (address[] memory)
    {
        uint256 total = restaking.operatorCount();
        address[] memory temp = new address[](total);
        uint256 count = 0;

        for (uint256 i = 0; i < total; i++) {
            address operator = restaking.operatorAt(i);
            if (_isEligibleOperator(restaking, blueprintId_, operator)) {
                temp[count] = operator;
                count++;
            }
        }

        address[] memory eligible = new address[](count);
        for (uint256 i = 0; i < count; i++) {
            eligible[i] = temp[i];
        }
        return eligible;
    }

    function isEligibleOperator(IMultiAssetDelegation restaking, uint256 blueprintId_, address operator)
        external
        view
        returns (bool)
    {
        return _isEligibleOperator(restaking, blueprintId_, operator);
    }

    function _selectOperators(
        address[] memory eligibleOps,
        uint32 operatorCount,
        bytes32 seed,
        uint32 minOperators_,
        uint32 maxOperators_
    ) internal pure returns (address[] memory) {
        uint256 available = eligibleOps.length;

        if (operatorCount < minOperators_) {
            revert InvalidOperatorCount(operatorCount, minOperators_, maxOperators_);
        }
        if (maxOperators_ > 0 && operatorCount > maxOperators_) {
            revert InvalidOperatorCount(operatorCount, minOperators_, maxOperators_);
        }
        if (operatorCount > available) {
            revert NotEnoughEligibleOperators(operatorCount, uint32(available));
        }

        // Work on a mutable copy so the caller's array isn't shuffled.
        address[] memory working = new address[](available);
        for (uint256 i = 0; i < available; i++) {
            working[i] = eligibleOps[i];
        }

        address[] memory selected = new address[](operatorCount);
        uint256 remaining = available;
        for (uint256 i = 0; i < operatorCount; i++) {
            uint256 rand = uint256(keccak256(abi.encode(seed, i))) % remaining;
            selected[i] = working[rand];
            working[rand] = working[remaining - 1];
            remaining -= 1;
        }
        return selected;
    }

    function _isEligibleOperator(IMultiAssetDelegation restaking, uint256 blueprintId_, address operator)
        internal
        view
        returns (bool)
    {
        if (!restaking.isOperatorActive(operator)) return false;
        uint256[] memory blueprints = restaking.getOperatorBlueprints(operator);
        for (uint256 i = 0; i < blueprints.length; i++) {
            if (blueprints[i] == blueprintId_) return true;
        }
        return false;
    }
}
