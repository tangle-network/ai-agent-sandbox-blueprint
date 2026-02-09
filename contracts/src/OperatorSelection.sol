// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/BlueprintServiceManagerBase.sol";
import "tnt-core/interfaces/IMultiAssetDelegation.sol";

/// @title OperatorSelectionBase
/// @notice Deterministic operator selection + validation helper for blueprint service requests.
abstract contract OperatorSelectionBase is BlueprintServiceManagerBase {
    IMultiAssetDelegation public restaking;

    uint32 public minOperators;
    uint32 public maxOperators;
    uint32 public defaultOperatorCount;

    event RestakingUpdated(address indexed restaking);
    event OperatorSelectionConfigUpdated(uint32 minOperators, uint32 maxOperators, uint32 defaultOperatorCount);

    error RestakingNotSet();
    error InvalidOperatorCount(uint32 requested, uint32 minOperators, uint32 maxOperators);
    error NotEnoughEligibleOperators(uint32 requested, uint32 available);
    error InvalidOperatorSelection();

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

    function setOperatorSelectionConfig(
        uint32 minOperators_,
        uint32 maxOperators_,
        uint32 defaultOperatorCount_
    ) external onlyBlueprintOwner {
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

    function previewOperatorSelection(uint32 operatorCount, bytes32 seed)
        public
        view
        returns (address[] memory)
    {
        return _selectOperators(operatorCount, seed);
    }

    function _decodeSelectionRequest(bytes calldata requestInputs)
        internal
        pure
        returns (SelectionRequest memory)
    {
        if (requestInputs.length == 0) {
            return SelectionRequest({ operatorCount: 0, seed: bytes32(0), enforceDeterministic: false });
        }
        return abi.decode(requestInputs, (SelectionRequest));
    }

    function _validateOperatorSelection(
        address[] calldata operators,
        SelectionRequest memory selection
    ) internal view {
        uint32 expectedCount = selection.operatorCount;
        if (expectedCount == 0) {
            expectedCount = defaultOperatorCount;
        }
        if (expectedCount == 0) {
            expectedCount = uint32(operators.length);
        }

        _validateOperatorCount(expectedCount);
        if (operators.length != expectedCount) {
            revert InvalidOperatorCount(uint32(operators.length), minOperators, maxOperators);
        }

        if (selection.enforceDeterministic) {
            address[] memory expected = _selectOperators(expectedCount, selection.seed);
            if (expected.length != operators.length) revert InvalidOperatorSelection();
            for (uint256 i = 0; i < expected.length; i++) {
                if (expected[i] != operators[i]) revert InvalidOperatorSelection();
            }
            return;
        }

        _validateOperators(operators);
    }

    function _validateOperatorCount(uint32 count) internal view {
        if (count < minOperators) {
            revert InvalidOperatorCount(count, minOperators, maxOperators);
        }
        if (maxOperators > 0 && count > maxOperators) {
            revert InvalidOperatorCount(count, minOperators, maxOperators);
        }
    }

    function _validateOperators(address[] calldata operators) internal view {
        if (address(restaking) == address(0)) revert RestakingNotSet();

        for (uint256 i = 0; i < operators.length; i++) {
            address operator = operators[i];
            if (!_isEligibleOperator(operator)) {
                revert InvalidOperatorSelection();
            }
            for (uint256 j = i + 1; j < operators.length; j++) {
                if (operators[j] == operator) {
                    revert InvalidOperatorSelection();
                }
            }
        }
    }

    function _selectOperators(uint32 operatorCount, bytes32 seed) internal view returns (address[] memory) {
        if (address(restaking) == address(0)) revert RestakingNotSet();

        address[] memory eligible = _eligibleOperators();
        uint256 available = eligible.length;

        if (operatorCount == 0) {
            operatorCount = defaultOperatorCount;
        }
        if (operatorCount == 0) {
            operatorCount = uint32(available);
        }

        _validateOperatorCount(operatorCount);

        if (operatorCount > available) {
            revert NotEnoughEligibleOperators(operatorCount, uint32(available));
        }

        address[] memory selected = new address[](operatorCount);
        uint256 remaining = available;

        for (uint256 i = 0; i < operatorCount; i++) {
            uint256 rand = uint256(keccak256(abi.encode(seed, i))) % remaining;
            selected[i] = eligible[rand];
            eligible[rand] = eligible[remaining - 1];
            remaining -= 1;
        }

        return selected;
    }

    function eligibleOperators() public view returns (address[] memory) {
        return _eligibleOperators();
    }

    function _eligibleOperators() internal view returns (address[] memory) {
        uint256 total = restaking.operatorCount();
        address[] memory temp = new address[](total);
        uint256 count = 0;

        for (uint256 i = 0; i < total; i++) {
            address operator = restaking.operatorAt(i);
            if (_isEligibleOperator(operator)) {
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

    function _isEligibleOperator(address operator) internal view returns (bool) {
        if (!restaking.isOperatorActive(operator)) {
            return false;
        }
        uint256[] memory blueprints = restaking.getOperatorBlueprints(operator);
        for (uint256 i = 0; i < blueprints.length; i++) {
            if (blueprints[i] == blueprintId) {
                return true;
            }
        }
        return false;
    }
}
