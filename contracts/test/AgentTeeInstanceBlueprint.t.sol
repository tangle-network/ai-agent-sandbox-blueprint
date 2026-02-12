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

        // Use literal job ID (0 = PROVISION) to avoid staticcall consuming expectRevert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(
                AgentTeeInstanceBlueprint.MissingTeeAttestation.selector,
                testServiceId,
                operator1
            )
        );
        teeInstance.onJobResult(
            testServiceId,
            0, // JOB_PROVISION
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
    // TEE PRICING
    // ═══════════════════════════════════════════════════════════════════════════

    function test_teePricingConstants() public view {
        assertEq(teeInstance.PRICE_MULT_PROVISION(), 500);
        assertEq(teeInstance.PRICE_MULT_TASK(), 350);
        assertEq(teeInstance.PRICE_MULT_PROMPT(), 30);
        assertEq(teeInstance.PRICE_MULT_EXEC(), 2);
        assertEq(teeInstance.PRICE_MULT_SNAPSHOT(), 10);
        assertEq(teeInstance.PRICE_MULT_SSH_PROVISION(), 3);
        assertEq(teeInstance.PRICE_MULT_SSH_REVOKE(), 1);
        assertEq(teeInstance.PRICE_MULT_DEPROVISION(), 5);
    }

    function test_teeGetDefaultJobRates() public view {
        uint256 baseRate = 0.001 ether;
        (uint8[] memory jobs, uint256[] memory rates) = teeInstance.getDefaultJobRates(baseRate);

        assertEq(jobs.length, 8);
        assertEq(rates[0], baseRate * 500); // PROVISION
        assertEq(rates[1], baseRate * 2);   // EXEC
        assertEq(rates[2], baseRate * 30);  // PROMPT
        assertEq(rates[3], baseRate * 350); // TASK
        assertEq(rates[4], baseRate * 3);   // SSH_PROVISION
        assertEq(rates[5], baseRate * 1);   // SSH_REVOKE
        assertEq(rates[6], baseRate * 10);  // SNAPSHOT
        assertEq(rates[7], baseRate * 5);   // DEPROVISION
    }

    function test_teeUnknownJobPriceReturnsZero() public view {
        assertEq(teeInstance.getJobPriceMultiplier(255), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INHERITED BEHAVIOR — LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    function test_provisionDeprovisionLifecycle() public {
        _provisionOperator(operator1);
        assertTrue(teeInstance.isProvisioned(testServiceId));

        // Deprovision
        uint64 callId = 200;
        simulateJobCall(testServiceId, teeInstance.JOB_DEPROVISION(), callId, bytes(""));

        vm.expectEmit(true, true, false, false);
        emit AgentTeeInstanceBlueprint.OperatorDeprovisioned(testServiceId, operator1);

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

    function test_jobGating() public {
        // Before provision, non-provision jobs should revert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentTeeInstanceBlueprint.NoOperatorsProvisioned.selector, testServiceId)
        );
        teeInstance.onJobCall(testServiceId, 1, 300, bytes("")); // JOB_EXEC = 1

        // After provision, should succeed
        _provisionOperator(operator1);
        simulateJobCall(testServiceId, teeInstance.JOB_EXEC(), 301, bytes(""));
    }

    function test_resultHashes() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);

        uint64 callId = 400;
        bytes memory out1 = encodeJsonOutputs('{"response":"from-op1"}');
        bytes memory out2 = encodeJsonOutputs('{"response":"from-op2"}');

        simulateJobCall(testServiceId, teeInstance.JOB_PROMPT(), callId, bytes(""));
        simulateJobResult(testServiceId, teeInstance.JOB_PROMPT(), callId, operator1, bytes(""), out1);
        simulateJobResult(testServiceId, teeInstance.JOB_PROMPT(), callId, operator2, bytes(""), out2);

        (address[] memory ops, bytes32[] memory hashes) = teeInstance.getJobResultHashes(testServiceId, callId);
        assertEq(ops.length, 2);
        assertEq(hashes[0], keccak256(out1));
        assertEq(hashes[1], keccak256(out2));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function test_blueprintName() public view {
        assertEq(teeInstance.BLUEPRINT_NAME(), "ai-agent-tee-instance-blueprint");
    }

    function test_blueprintVersion() public view {
        assertEq(teeInstance.BLUEPRINT_VERSION(), "0.1.0");
    }
}
