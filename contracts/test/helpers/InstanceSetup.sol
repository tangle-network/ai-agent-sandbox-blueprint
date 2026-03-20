// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Test.sol";
import "../../src/AgentSandboxBlueprint.sol";

/// @dev Minimal tangle core mock for direct instance reporting auth checks.
contract MockTangleCoreInstance {
    mapping(uint64 => mapping(address => bool)) private _serviceOperators;

    function setServiceOperator(uint64 serviceId, address operator, bool active) external {
        _serviceOperators[serviceId][operator] = active;
    }

    function isServiceOperator(uint64 serviceId, address operator) external view returns (bool) {
        return _serviceOperators[serviceId][operator];
    }
}

/// @title InstanceBlueprintTestSetup
/// @dev Base test contract providing helpers for instance mode tests.
///      Uses the unified AgentSandboxBlueprint with instanceMode=true.
contract InstanceBlueprintTestSetup is Test {
    AgentSandboxBlueprint public instance;
    MockTangleCoreInstance internal tangleMock;

    address public tangleCore;
    address public blueprintOwner = address(0xBB);
    uint64 public testBlueprintId = 42;

    address public operator1 = address(0x1001);
    address public operator2 = address(0x1002);
    address public operator3 = address(0x1003);

    uint64 public testServiceId = 1;

    function setUp() public virtual {
        tangleMock = new MockTangleCoreInstance();
        tangleCore = address(tangleMock);
        instance = new AgentSandboxBlueprint(address(0), true, false);
        instance.onBlueprintCreated(testBlueprintId, blueprintOwner, tangleCore);
    }

    function setServiceOperator(uint64 serviceId, address operator, bool active) internal {
        tangleMock.setServiceOperator(serviceId, operator, active);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB SIMULATION HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    function simulateJobCall(uint64 serviceId, uint8 jobIndex, uint64 callId, bytes memory inputs) internal {
        vm.prank(tangleCore);
        instance.onJobCall(serviceId, jobIndex, callId, inputs);
    }

    function simulateJobResult(
        uint64 serviceId,
        uint8 jobIndex,
        uint64 callId,
        address operator,
        bytes memory inputs,
        bytes memory outputs
    ) internal {
        vm.prank(tangleCore);
        instance.onJobResult(serviceId, jobIndex, callId, operator, inputs, outputs);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // ENCODING HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    /// @dev Encode provision outputs: (string sandboxId, string sidecarUrl, uint32 sshPort, string teeAttestationJson).
    function encodeProvisionOutputs(
        string memory sandboxId,
        string memory sidecarUrl,
        uint32 sshPort,
        string memory teeAttestationJson
    ) internal pure returns (bytes memory) {
        return abi.encode(sandboxId, sidecarUrl, sshPort, teeAttestationJson);
    }

    /// @dev Encode generic JSON outputs: (string json).
    function encodeJsonOutputs(string memory json) internal pure returns (bytes memory) {
        return abi.encode(json);
    }

    /// @dev Encode workflow create inputs in the same flat ABI shape the UI/operator submit.
    function encodeWorkflowCreateInputs(AgentSandboxBlueprint.WorkflowCreateRequest memory request)
        internal
        pure
        returns (bytes memory)
    {
        return abi.encode(
            request.name,
            request.workflow_json,
            request.trigger_type,
            request.trigger_config,
            request.sandbox_config_json
        );
    }

    /// @dev Full provision flow via operator-direct lifecycle reporting.
    function _provisionOperator(address operator) internal {
        _provisionOperatorFull(operator, "http://sidecar:8080", 2222, "");
    }

    /// @dev Full provision flow with custom sidecar URL and attestation.
    function _provisionOperatorFull(
        address operator,
        string memory sidecarUrl,
        uint32 sshPort,
        string memory attestation
    ) internal {
        setServiceOperator(testServiceId, operator, true);
        vm.prank(operator);
        instance.reportProvisioned(
            testServiceId, string(abi.encodePacked("sb-", vm.toString(operator))), sidecarUrl, sshPort, attestation
        );
    }

    /// @dev Full deprovision flow via operator-direct lifecycle reporting.
    function _deprovisionOperator(address operator) internal {
        vm.prank(operator);
        instance.reportDeprovisioned(testServiceId);
    }
}

/// @title TeeInstanceBlueprintTestSetup
/// @dev Base test for the TEE variant. Uses unified contract with teeRequired=true.
contract TeeInstanceBlueprintTestSetup is Test {
    AgentSandboxBlueprint public teeInstance;
    MockTangleCoreInstance internal tangleMock;

    address public tangleCore;
    address public blueprintOwner = address(0xBB);
    uint64 public testBlueprintId = 42;

    address public operator1 = address(0x1001);
    address public operator2 = address(0x1002);

    uint64 public testServiceId = 1;

    function setUp() public virtual {
        tangleMock = new MockTangleCoreInstance();
        tangleCore = address(tangleMock);
        teeInstance = new AgentSandboxBlueprint(address(0), true, true);
        teeInstance.onBlueprintCreated(testBlueprintId, blueprintOwner, tangleCore);
    }

    function setServiceOperator(uint64 serviceId, address operator, bool active) internal {
        tangleMock.setServiceOperator(serviceId, operator, active);
    }

    function simulateJobCall(uint64 serviceId, uint8 jobIndex, uint64 callId, bytes memory inputs) internal {
        vm.prank(tangleCore);
        teeInstance.onJobCall(serviceId, jobIndex, callId, inputs);
    }

    function simulateJobResult(
        uint64 serviceId,
        uint8 jobIndex,
        uint64 callId,
        address operator,
        bytes memory inputs,
        bytes memory outputs
    ) internal {
        vm.prank(tangleCore);
        teeInstance.onJobResult(serviceId, jobIndex, callId, operator, inputs, outputs);
    }

    function encodeProvisionOutputs(
        string memory sandboxId,
        string memory sidecarUrl,
        uint32 sshPort,
        string memory teeAttestationJson
    ) internal pure returns (bytes memory) {
        return abi.encode(sandboxId, sidecarUrl, sshPort, teeAttestationJson);
    }

    function encodeJsonOutputs(string memory json) internal pure returns (bytes memory) {
        return abi.encode(json);
    }

    /// @dev Full provision flow with mandatory attestation (TEE requires it).
    function _provisionOperator(address operator) internal {
        _provisionOperatorFull(operator, "http://sidecar:8080", 2222, '{"tee":"phala","quote":"abc123"}');
    }

    function _provisionOperatorFull(
        address operator,
        string memory sidecarUrl,
        uint32 sshPort,
        string memory attestation
    ) internal {
        setServiceOperator(testServiceId, operator, true);
        vm.prank(operator);
        teeInstance.reportProvisioned(
            testServiceId, string(abi.encodePacked("sb-", vm.toString(operator))), sidecarUrl, sshPort, attestation
        );
    }

    function _deprovisionOperator(address operator) internal {
        vm.prank(operator);
        teeInstance.reportDeprovisioned(testServiceId);
    }
}
