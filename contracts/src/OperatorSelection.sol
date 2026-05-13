// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/BlueprintServiceManagerBase.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";
import "./libraries/OperatorSelectionLib.sol";

/// @title OperatorSelectionBase
/// @notice Deterministic operator selection + validation helper for
///         blueprint service requests. The compute-heavy paths
///         (`_selectOperators`, `_eligibleOperators`, the validation walk)
///         live in `OperatorSelectionLib` so the inheriting blueprint
///         contract stays under the EIP-170 24,576 B runtime cap.
abstract contract OperatorSelectionBase is BlueprintServiceManagerBase {
    IMultiAssetDelegation public restaking;

    uint32 public minOperators;
    uint32 public maxOperators;
    uint32 public defaultOperatorCount;

    event RestakingUpdated(address indexed restaking);
    event OperatorSelectionConfigUpdated(uint32 minOperators, uint32 maxOperators, uint32 defaultOperatorCount);

    error RestakingNotSet();
    error InvalidOperatorCount(uint32 requested, uint32 minOperators, uint32 maxOperators);

    struct SelectionRequest {
        uint32 operatorCount;
        bytes32 seed;
        bool enforceDeterministic;
    }

    function setRestaking(address restakingAddress) external onlyBlueprintOwner {
        if (restakingAddress == address(0)) revert RestakingNotSet();
        restaking = IMultiAssetDelegation(restakingAddress);
        emit RestakingUpdated(restakingAddress);
    }

    function setOperatorSelectionConfig(uint32 minOperators_, uint32 maxOperators_, uint32 defaultOperatorCount_)
        external
        onlyBlueprintOwner
    {
        if (maxOperators_ > 0 && minOperators_ > maxOperators_) {
            revert InvalidOperatorCount(minOperators_, minOperators_, maxOperators_);
        }
        if (defaultOperatorCount_ > 0) {
            if (defaultOperatorCount_ < minOperators_) {
                revert InvalidOperatorCount(defaultOperatorCount_, minOperators_, maxOperators_);
            }
            if (maxOperators_ > 0 && defaultOperatorCount_ > maxOperators_) {
                revert InvalidOperatorCount(defaultOperatorCount_, minOperators_, maxOperators_);
            }
        }

        minOperators = minOperators_;
        maxOperators = maxOperators_;
        defaultOperatorCount = defaultOperatorCount_;

        emit OperatorSelectionConfigUpdated(minOperators_, maxOperators_, defaultOperatorCount_);
    }

    function previewOperatorSelection(uint32 operatorCount, bytes32 seed) public view returns (address[] memory) {
        if (address(restaking) == address(0)) revert RestakingNotSet();
        uint32 count = operatorCount == 0 ? defaultOperatorCount : operatorCount;
        if (count == 0) count = uint32(OperatorSelectionLib.eligibleOperators(restaking, blueprintId).length);
        return OperatorSelectionLib.selectOperators(
            OperatorSelectionLib.eligibleOperators(restaking, blueprintId), count, seed, minOperators, maxOperators
        );
    }

    function _decodeSelectionRequest(bytes calldata requestInputs) internal pure returns (SelectionRequest memory) {
        if (requestInputs.length == 0) {
            return SelectionRequest({operatorCount: 0, seed: bytes32(0), enforceDeterministic: false});
        }
        return abi.decode(requestInputs, (SelectionRequest));
    }

    function _validateOperatorSelection(address[] calldata operators, SelectionRequest memory selection) internal view {
        if (address(restaking) == address(0)) revert RestakingNotSet();

        uint32 expectedCount = selection.operatorCount;
        if (expectedCount == 0) expectedCount = defaultOperatorCount;
        if (expectedCount == 0) expectedCount = uint32(operators.length);

        if (expectedCount < minOperators) {
            revert InvalidOperatorCount(expectedCount, minOperators, maxOperators);
        }
        if (maxOperators > 0 && expectedCount > maxOperators) {
            revert InvalidOperatorCount(expectedCount, minOperators, maxOperators);
        }

        OperatorSelectionLib.validateOperatorSelection(
            operators,
            OperatorSelectionLib.eligibleOperators(restaking, blueprintId),
            minOperators,
            maxOperators,
            expectedCount,
            selection.seed,
            selection.enforceDeterministic
        );
    }

    function eligibleOperators() public view returns (address[] memory) {
        if (address(restaking) == address(0)) revert RestakingNotSet();
        return OperatorSelectionLib.eligibleOperators(restaking, blueprintId);
    }

    function _eligibleOperators() internal view returns (address[] memory) {
        if (address(restaking) == address(0)) revert RestakingNotSet();
        return OperatorSelectionLib.eligibleOperators(restaking, blueprintId);
    }

    function _isEligibleOperator(address operator) internal view returns (bool) {
        return OperatorSelectionLib.isEligibleOperator(restaking, blueprintId, operator);
    }
}
