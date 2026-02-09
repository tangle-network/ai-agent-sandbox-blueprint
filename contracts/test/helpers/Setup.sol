// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Test.sol";
import "../../src/AgentSandboxBlueprint.sol";

/// @title MockMultiAssetDelegation
/// @dev Minimal mock of IMultiAssetDelegation for testing operator selection and capacity.
contract MockMultiAssetDelegation {
    address[] private _operators;
    mapping(address => bool) private _active;
    mapping(address => uint256[]) private _blueprints;

    function addOperator(address operator, uint256 blueprintId) external {
        if (!_isInList(operator)) {
            _operators.push(operator);
        }
        _active[operator] = true;
        _blueprints[operator].push(blueprintId);
    }

    function setActive(address operator, bool active) external {
        _active[operator] = active;
    }

    function operatorCount() external view returns (uint256) {
        return _operators.length;
    }

    function operatorAt(uint256 index) external view returns (address) {
        return _operators[index];
    }

    function isOperatorActive(address operator) external view returns (bool) {
        return _active[operator];
    }

    function getOperatorBlueprints(address operator) external view returns (uint256[] memory) {
        return _blueprints[operator];
    }

    function _isInList(address operator) internal view returns (bool) {
        for (uint256 i = 0; i < _operators.length; i++) {
            if (_operators[i] == operator) return true;
        }
        return false;
    }
}

/// @title BlueprintTestSetup
/// @dev Base test contract providing helpers for AgentSandboxBlueprint tests.
contract BlueprintTestSetup is Test {
    AgentSandboxBlueprint public blueprint;
    MockMultiAssetDelegation public mockDelegation;

    address public tangleCore = address(0x7A);
    address public blueprintOwner = address(0xBB);
    uint64 public testBlueprintId = 42;

    address public operator1 = address(0x1001);
    address public operator2 = address(0x1002);
    address public operator3 = address(0x1003);

    function setUp() public virtual {
        mockDelegation = new MockMultiAssetDelegation();
        blueprint = new AgentSandboxBlueprint(address(mockDelegation));
        // Initialize blueprint via onBlueprintCreated
        blueprint.onBlueprintCreated(testBlueprintId, blueprintOwner, tangleCore);
    }

    /// @dev Register an operator in the mock delegation and on the blueprint.
    function registerOperator(address operator, uint32 capacity) internal {
        mockDelegation.addOperator(operator, testBlueprintId);
        bytes memory registrationInputs = capacity > 0
            ? abi.encode(capacity)
            : bytes("");
        vm.prank(tangleCore);
        blueprint.onRegister(operator, registrationInputs);
    }

    /// @dev Simulate onJobCall as tangleCore.
    function simulateJobCall(
        uint64 serviceId,
        uint8 jobIndex,
        uint64 callId,
        bytes memory inputs
    ) internal {
        vm.prank(tangleCore);
        blueprint.onJobCall(serviceId, jobIndex, callId, inputs);
    }

    /// @dev Simulate onJobResult as tangleCore.
    function simulateJobResult(
        uint64 serviceId,
        uint8 jobIndex,
        uint64 callId,
        address operator,
        bytes memory inputs,
        bytes memory outputs
    ) internal {
        vm.prank(tangleCore);
        blueprint.onJobResult(serviceId, jobIndex, callId, operator, inputs, outputs);
    }

    /// @dev Encode sandbox create inputs (empty for create — no sandboxId needed on input).
    function encodeSandboxCreateInputs() internal pure returns (bytes memory) {
        return bytes("");
    }

    /// @dev Encode sandbox create outputs: (string sandboxId, string json).
    function encodeSandboxCreateOutputs(
        string memory sandboxId,
        string memory json
    ) internal pure returns (bytes memory) {
        return abi.encode(sandboxId, json);
    }

    /// @dev Encode sandbox ID inputs for stop/resume/delete: (string sandboxId).
    function encodeSandboxIdInputs(string memory sandboxId) internal pure returns (bytes memory) {
        return abi.encode(sandboxId);
    }

    /// @dev Encode generic JSON outputs for non-create jobs: (string json).
    function encodeJsonOutputs(string memory json) internal pure returns (bytes memory) {
        return abi.encode(json);
    }
}
