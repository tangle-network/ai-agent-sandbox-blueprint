// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Test.sol";
import "../../src/AgentSandboxBlueprint.sol";

/// @title InstanceBlueprintTestSetup
/// @dev Base test contract providing helpers for instance mode tests.
///      Uses the unified AgentSandboxBlueprint with instanceMode=true.
contract InstanceBlueprintTestSetup is Test {
    AgentSandboxBlueprint public instance;

    address public tangleCore = address(0x7A);
    address public blueprintOwner = address(0xBB);
    uint64 public testBlueprintId = 42;

    address public operator1 = address(0x1001);
    address public operator2 = address(0x1002);
    address public operator3 = address(0x1003);

    uint64 public testServiceId = 1;

    function setUp() public virtual {
        instance = new AgentSandboxBlueprint(address(0), true, false);
        instance.onBlueprintCreated(testBlueprintId, blueprintOwner, tangleCore);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB SIMULATION HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    function simulateJobCall(
        uint64 serviceId,
        uint8 jobIndex,
        uint64 callId,
        bytes memory inputs
    ) internal {
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

    /// @dev Full provision flow: jobCall + jobResult for a single operator.
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
        uint64 callId = uint64(uint160(operator)); // unique per operator
        simulateJobCall(testServiceId, instance.JOB_PROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_PROVISION(),
            callId,
            operator,
            bytes(""),
            encodeProvisionOutputs(
                string(abi.encodePacked("sb-", vm.toString(operator))),
                sidecarUrl,
                sshPort,
                attestation
            )
        );
    }
}

/// @title TeeInstanceBlueprintTestSetup
/// @dev Base test for the TEE variant. Uses unified contract with teeRequired=true.
contract TeeInstanceBlueprintTestSetup is Test {
    AgentSandboxBlueprint public teeInstance;

    address public tangleCore = address(0x7A);
    address public blueprintOwner = address(0xBB);
    uint64 public testBlueprintId = 42;

    address public operator1 = address(0x1001);
    address public operator2 = address(0x1002);

    uint64 public testServiceId = 1;

    function setUp() public virtual {
        teeInstance = new AgentSandboxBlueprint(address(0), true, true);
        teeInstance.onBlueprintCreated(testBlueprintId, blueprintOwner, tangleCore);
    }

    function simulateJobCall(
        uint64 serviceId,
        uint8 jobIndex,
        uint64 callId,
        bytes memory inputs
    ) internal {
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
        uint64 callId = uint64(uint160(operator));
        simulateJobCall(testServiceId, teeInstance.JOB_PROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            teeInstance.JOB_PROVISION(),
            callId,
            operator,
            bytes(""),
            encodeProvisionOutputs(
                string(abi.encodePacked("sb-", vm.toString(operator))),
                sidecarUrl,
                sshPort,
                attestation
            )
        );
    }
}
