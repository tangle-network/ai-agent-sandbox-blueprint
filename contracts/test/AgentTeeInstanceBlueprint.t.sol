// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./helpers/InstanceSetup.sol";

contract AgentTeeInstanceBlueprintTest is TeeInstanceBlueprintTestSetup {

    // ═══════════════════════════════════════════════════════════════════════════
    // ATTESTATION ENFORCEMENT
    // ═══════════════════════════════════════════════════════════════════════════

    function test_provisionWithEmptyAttestationReverts() public {
        uint64 callId = 100;
        simulateJobCall(testServiceId, teeInstance.JOB_PROVISION(), callId, bytes(""));

        // Use literal job ID (5 = PROVISION)
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(
                AgentSandboxBlueprint.MissingTeeAttestation.selector,
                testServiceId,
                operator1
            )
        );
        teeInstance.onJobResult(
            testServiceId,
            5, // JOB_PROVISION
            callId,
            operator1,
            bytes(""),
            encodeProvisionOutputs("sb-1", "http://sidecar:8080", 2222, "")
        );
    }

    function test_provisionWithAttestationSucceeds() public {
        _provisionOperator(operator1);

        assertTrue(teeInstance.isProvisioned(testServiceId));
        assertTrue(teeInstance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(teeInstance.getOperatorCount(testServiceId), 1);
    }

    function test_attestationHashAlwaysStored() public {
        string memory attestation = '{"tee":"phala","quote":"abc123"}';
        _provisionOperator(operator1); // uses default attestation

        bytes32 expectedHash = keccak256(bytes(attestation));
        assertEq(teeInstance.getAttestationHash(testServiceId, operator1), expectedHash);
        assertTrue(teeInstance.getAttestationHash(testServiceId, operator1) != bytes32(0));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MODE FLAGS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_teeModeFlagsSet() public view {
        assertTrue(teeInstance.instanceMode());
        assertTrue(teeInstance.teeRequired());
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    function test_provisionDeprovisionLifecycle() public {
        _provisionOperator(operator1);
        assertTrue(teeInstance.isProvisioned(testServiceId));

        uint64 callId = 200;
        simulateJobCall(testServiceId, teeInstance.JOB_DEPROVISION(), callId, bytes(""));

        vm.expectEmit(true, true, false, false);
        emit AgentSandboxBlueprint.OperatorDeprovisioned(testServiceId, operator1);

        simulateJobResult(
            testServiceId,
            teeInstance.JOB_DEPROVISION(),
            callId,
            operator1,
            bytes(""),
            encodeJsonOutputs("{}")
        );

        assertFalse(teeInstance.isProvisioned(testServiceId));
        assertEq(teeInstance.getOperatorCount(testServiceId), 0);
    }

    function test_multiOperator() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);

        assertEq(teeInstance.getOperatorCount(testServiceId), 2);

        (address[] memory ops, string[] memory urls) = teeInstance.getOperatorEndpoints(testServiceId);
        assertEq(ops.length, 2);
        assertEq(urls.length, 2);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function test_blueprintName() public view {
        assertEq(teeInstance.BLUEPRINT_NAME(), "ai-agent-sandbox-blueprint");
    }

    function test_blueprintVersion() public view {
        assertEq(teeInstance.BLUEPRINT_VERSION(), "0.4.0");
    }
}
