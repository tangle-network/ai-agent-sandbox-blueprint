// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./helpers/InstanceSetup.sol";

contract AgentInstanceBlueprintTest is InstanceBlueprintTestSetup {

    // ═══════════════════════════════════════════════════════════════════════════
    // PROVISION FLOW
    // ═══════════════════════════════════════════════════════════════════════════

    function test_provisionSingleOperator() public {
        _provisionOperator(operator1);

        assertTrue(instance.isProvisioned(testServiceId));
        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(instance.getOperatorCount(testServiceId), 1);
    }

    function test_provisionStoresSidecarUrl() public {
        _provisionOperatorFull(operator1, "http://my-sidecar:9090", 3333, "");

        assertEq(instance.operatorSidecarUrl(testServiceId, operator1), "http://my-sidecar:9090");
    }

    function test_provisionEmitsEvent() public {
        uint64 callId = uint64(uint160(operator1));

        simulateJobCall(testServiceId, instance.JOB_PROVISION(), callId, bytes(""));

        vm.expectEmit(true, true, false, true);
        emit AgentInstanceBlueprint.OperatorProvisioned(
            testServiceId,
            operator1,
            string(abi.encodePacked("sb-", vm.toString(operator1))),
            "http://sidecar:8080"
        );

        simulateJobResult(
            testServiceId,
            instance.JOB_PROVISION(),
            callId,
            operator1,
            bytes(""),
            encodeProvisionOutputs(
                string(abi.encodePacked("sb-", vm.toString(operator1))),
                "http://sidecar:8080",
                2222,
                ""
            )
        );
    }

    function test_provisionAlreadyProvisionedReverts() public {
        _provisionOperator(operator1);

        uint64 callId = 999;
        simulateJobCall(testServiceId, instance.JOB_PROVISION(), callId, bytes(""));

        // Use literal job ID (0 = PROVISION) to avoid staticcall consuming expectRevert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentInstanceBlueprint.AlreadyProvisioned.selector, testServiceId, operator1)
        );
        instance.onJobResult(
            testServiceId,
            0, // JOB_PROVISION
            callId,
            operator1,
            bytes(""),
            encodeProvisionOutputs("sb-dup", "http://dup:8080", 2222, "")
        );
    }

    function test_provisionWithAttestationStoresHash() public {
        string memory attestation = '{"tee":"nitro","quote":"deadbeef"}';
        _provisionOperatorFull(operator1, "http://sidecar:8080", 2222, attestation);

        bytes32 expectedHash = keccak256(bytes(attestation));
        assertEq(instance.getAttestationHash(testServiceId, operator1), expectedHash);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // DEPROVISION FLOW
    // ═══════════════════════════════════════════════════════════════════════════

    function test_deprovisionCleanRemoval() public {
        _provisionOperator(operator1);
        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));

        // Deprovision
        uint64 callId = 500;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));

        vm.expectEmit(true, true, false, false);
        emit AgentInstanceBlueprint.OperatorDeprovisioned(testServiceId, operator1);

        simulateJobResult(
            testServiceId,
            instance.JOB_DEPROVISION(),
            callId,
            operator1,
            bytes(""),
            encodeJsonOutputs("{}")
        );

        assertFalse(instance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertFalse(instance.isProvisioned(testServiceId));
    }

    function test_deprovisionNotProvisionedReverts() public {
        uint64 callId = 501;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));

        // Use literal job ID (7 = DEPROVISION) to avoid staticcall consuming expectRevert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentInstanceBlueprint.NotProvisioned.selector, testServiceId, operator1)
        );
        instance.onJobResult(
            testServiceId,
            7, // JOB_DEPROVISION
            callId,
            operator1,
            bytes(""),
            encodeJsonOutputs("{}")
        );
    }

    function test_deprovisionSwapAndPop() public {
        // Provision 3 operators
        _provisionOperator(operator1);
        _provisionOperator(operator2);
        _provisionOperator(operator3);

        assertEq(instance.getOperatorCount(testServiceId), 3);

        // Deprovision the middle one (operator2) — triggers swap-and-pop
        uint64 callId = 502;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_DEPROVISION(),
            callId,
            operator2,
            bytes(""),
            encodeJsonOutputs("{}")
        );

        assertEq(instance.getOperatorCount(testServiceId), 2);
        assertFalse(instance.isOperatorProvisioned(testServiceId, operator2));

        // Remaining operators should still work
        (address[] memory ops, string[] memory urls) = instance.getOperatorEndpoints(testServiceId);
        assertEq(ops.length, 2);
        assertEq(urls.length, 2);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB GATING
    // ═══════════════════════════════════════════════════════════════════════════

    function test_jobCallRevertsWhenNoOperatorsProvisioned() public {
        // Use literal job ID to avoid staticcall consuming expectRevert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentInstanceBlueprint.NoOperatorsProvisioned.selector, testServiceId)
        );
        instance.onJobCall(testServiceId, 1, 600, bytes("")); // JOB_EXEC = 1
    }

    function test_jobCallSucceedsAfterProvision() public {
        _provisionOperator(operator1);

        // JOB_EXEC should now succeed
        simulateJobCall(testServiceId, instance.JOB_EXEC(), 601, bytes(""));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MULTI-OPERATOR
    // ═══════════════════════════════════════════════════════════════════════════

    function test_twoOperatorsProvisionIndependently() public {
        _provisionOperatorFull(operator1, "http://op1:8080", 2222, "");
        _provisionOperatorFull(operator2, "http://op2:9090", 3333, "");

        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));
        assertTrue(instance.isOperatorProvisioned(testServiceId, operator2));
        assertEq(instance.getOperatorCount(testServiceId), 2);
    }

    function test_multiOperatorCorrectCount() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);
        _provisionOperator(operator3);

        assertEq(instance.getOperatorCount(testServiceId), 3);
    }

    function test_getOperatorEndpointsReturnsBoth() public {
        _provisionOperatorFull(operator1, "http://op1:8080", 2222, "");
        _provisionOperatorFull(operator2, "http://op2:9090", 3333, "");

        (address[] memory ops, string[] memory urls) = instance.getOperatorEndpoints(testServiceId);
        assertEq(ops.length, 2);
        assertEq(urls.length, 2);

        // Order: operator1 first, operator2 second
        assertEq(ops[0], operator1);
        assertEq(ops[1], operator2);
        assertEq(urls[0], "http://op1:8080");
        assertEq(urls[1], "http://op2:9090");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // RESULT HASHES
    // ═══════════════════════════════════════════════════════════════════════════

    function test_promptStoresResultHash() public {
        _provisionOperator(operator1);

        uint64 callId = 700;
        bytes memory promptOutputs = encodeJsonOutputs('{"response":"hello"}');
        bytes32 expectedHash = keccak256(promptOutputs);

        simulateJobCall(testServiceId, instance.JOB_PROMPT(), callId, bytes(""));

        vm.expectEmit(true, true, true, true);
        emit AgentInstanceBlueprint.OperatorResultSubmitted(
            testServiceId, callId, operator1, instance.JOB_PROMPT(), expectedHash
        );

        simulateJobResult(
            testServiceId,
            instance.JOB_PROMPT(),
            callId,
            operator1,
            bytes(""),
            promptOutputs
        );

        assertEq(instance.jobResultHash(testServiceId, callId, operator1), expectedHash);
    }

    function test_taskStoresResultHash() public {
        _provisionOperator(operator1);

        uint64 callId = 701;
        bytes memory taskOutputs = encodeJsonOutputs('{"result":"done"}');
        bytes32 expectedHash = keccak256(taskOutputs);

        simulateJobCall(testServiceId, instance.JOB_TASK(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_TASK(),
            callId,
            operator1,
            bytes(""),
            taskOutputs
        );

        assertEq(instance.jobResultHash(testServiceId, callId, operator1), expectedHash);
    }

    function test_execDoesNotStoreResultHash() public {
        _provisionOperator(operator1);

        uint64 callId = 702;
        simulateJobCall(testServiceId, instance.JOB_EXEC(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_EXEC(),
            callId,
            operator1,
            bytes(""),
            encodeJsonOutputs('{"stdout":"ok"}')
        );

        // Exec result hash should be zero (not stored)
        assertEq(instance.jobResultHash(testServiceId, callId, operator1), bytes32(0));
    }

    function test_getJobResultHashesEnumeration() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);

        uint64 callId = 703;
        bytes memory out1 = encodeJsonOutputs('{"response":"from-op1"}');
        bytes memory out2 = encodeJsonOutputs('{"response":"from-op2"}');

        simulateJobCall(testServiceId, instance.JOB_PROMPT(), callId, bytes(""));

        simulateJobResult(testServiceId, instance.JOB_PROMPT(), callId, operator1, bytes(""), out1);
        simulateJobResult(testServiceId, instance.JOB_PROMPT(), callId, operator2, bytes(""), out2);

        (address[] memory ops, bytes32[] memory hashes) = instance.getJobResultHashes(testServiceId, callId);
        assertEq(ops.length, 2);
        assertEq(hashes[0], keccak256(out1));
        assertEq(hashes[1], keccak256(out2));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REQUIRED RESULT COUNT
    // ═══════════════════════════════════════════════════════════════════════════

    function test_requiredResultCountForPromptAndTask() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);

        // Prompt and task require ALL operators
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_PROMPT()), 2);
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_TASK()), 2);
    }

    function test_requiredResultCountForOtherJobs() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);

        // All other jobs require just 1
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_EXEC()), 1);
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_SSH_PROVISION()), 1);
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_SNAPSHOT()), 1);
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_PROVISION()), 1);
        assertEq(instance.getRequiredResultCount(testServiceId, instance.JOB_DEPROVISION()), 1);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PRICING
    // ═══════════════════════════════════════════════════════════════════════════

    function test_getDefaultJobRates() public view {
        uint256 baseRate = 1 ether;
        (uint8[] memory jobs, uint256[] memory rates) = instance.getDefaultJobRates(baseRate);

        assertEq(jobs.length, 8);
        assertEq(rates.length, 8);

        // Verify each rate = baseRate * multiplier
        assertEq(rates[0], baseRate * 50);  // PROVISION
        assertEq(rates[1], baseRate * 1);   // EXEC
        assertEq(rates[2], baseRate * 20);  // PROMPT
        assertEq(rates[3], baseRate * 250); // TASK
        assertEq(rates[4], baseRate * 2);   // SSH_PROVISION
        assertEq(rates[5], baseRate * 1);   // SSH_REVOKE
        assertEq(rates[6], baseRate * 5);   // SNAPSHOT
        assertEq(rates[7], baseRate * 1);   // DEPROVISION
    }

    function test_getJobPriceMultiplier() public view {
        assertEq(instance.getJobPriceMultiplier(instance.JOB_PROVISION()), 50);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_EXEC()), 1);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_PROMPT()), 20);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_TASK()), 250);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_SSH_PROVISION()), 2);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_SSH_REVOKE()), 1);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_SNAPSHOT()), 5);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_DEPROVISION()), 1);
    }

    function test_unknownJobPriceReturnsZero() public view {
        assertEq(instance.getJobPriceMultiplier(255), 0);
        assertEq(instance.getJobPriceMultiplier(8), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_isProvisionedAndIsOperatorProvisioned() public {
        assertFalse(instance.isProvisioned(testServiceId));
        assertFalse(instance.isOperatorProvisioned(testServiceId, operator1));

        _provisionOperator(operator1);

        assertTrue(instance.isProvisioned(testServiceId));
        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));
        assertFalse(instance.isOperatorProvisioned(testServiceId, operator2));
    }

    function test_getAttestationHashWithNoAttestation() public {
        _provisionOperatorFull(operator1, "http://sidecar:8080", 2222, "");
        assertEq(instance.getAttestationHash(testServiceId, operator1), bytes32(0));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function test_jobMetadata() public view {
        uint8[] memory ids = instance.jobIds();
        assertEq(ids.length, 8);
        assertEq(ids[0], 0);
        assertEq(ids[7], 7);

        assertTrue(instance.supportsJob(0));
        assertTrue(instance.supportsJob(7));
        assertFalse(instance.supportsJob(8));

        assertEq(instance.jobCount(), 8);
    }

    function test_blueprintMetadata() public view {
        assertEq(instance.BLUEPRINT_NAME(), "ai-agent-instance-blueprint");
        assertEq(instance.BLUEPRINT_VERSION(), "0.3.0");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // FULL LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    function test_fullProvisionDeprovisionCycle() public {
        // 1. Provision
        _provisionOperator(operator1);
        assertTrue(instance.isProvisioned(testServiceId));
        assertEq(instance.getOperatorCount(testServiceId), 1);

        // 2. Run an exec job
        simulateJobCall(testServiceId, instance.JOB_EXEC(), 800, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_EXEC(),
            800,
            operator1,
            bytes(""),
            encodeJsonOutputs('{"stdout":"ok"}')
        );

        // 3. Run a prompt job (stores result hash)
        simulateJobCall(testServiceId, instance.JOB_PROMPT(), 801, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_PROMPT(),
            801,
            operator1,
            bytes(""),
            encodeJsonOutputs('{"response":"hello"}')
        );
        assertTrue(instance.jobResultHash(testServiceId, 801, operator1) != bytes32(0));

        // 4. Deprovision
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), 802, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_DEPROVISION(),
            802,
            operator1,
            bytes(""),
            encodeJsonOutputs("{}")
        );

        assertFalse(instance.isProvisioned(testServiceId));
        assertEq(instance.getOperatorCount(testServiceId), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE TERMINATION
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onServiceTerminationEmitsEvent() public {
        _provisionOperator(operator1);

        vm.expectEmit(true, true, false, false);
        emit AgentInstanceBlueprint.ServiceTerminationReceived(testServiceId, blueprintOwner);

        vm.prank(tangleCore);
        instance.onServiceTermination(testServiceId, blueprintOwner);
    }

    function test_onServiceTerminationOnlyTangle() public {
        vm.prank(operator1);
        vm.expectRevert();
        instance.onServiceTermination(testServiceId, blueprintOwner);
    }

    function test_fullLifecycleWithTermination() public {
        // 1. Provision two operators
        _provisionOperator(operator1);
        _provisionOperator(operator2);
        assertEq(instance.getOperatorCount(testServiceId), 2);

        // 2. Run exec job
        simulateJobCall(testServiceId, instance.JOB_EXEC(), 1000, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_EXEC(), 1000, operator1, bytes(""), encodeJsonOutputs('{"stdout":"ok"}')
        );

        // 3. Termination signal from Tangle
        vm.prank(tangleCore);
        instance.onServiceTermination(testServiceId, blueprintOwner);

        // 4. Deprovision both operators
        uint64 callId1 = 1001;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId1, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId1, operator1, bytes(""), encodeJsonOutputs("{}")
        );

        uint64 callId2 = 1002;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId2, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId2, operator2, bytes(""), encodeJsonOutputs("{}")
        );

        // 5. Verify cleanup
        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertFalse(instance.isProvisioned(testServiceId));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // EDGE CASE: Job result from non-provisioned operator
    // ═══════════════════════════════════════════════════════════════════════════

    function test_jobResultFromNonProvisionedOperatorReverts() public {
        _provisionOperator(operator1);

        // operator2 is NOT provisioned — submitting a result should revert
        // Use literal job ID (1 = EXEC) to avoid staticcall consuming expectRevert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentInstanceBlueprint.NotProvisioned.selector, testServiceId, operator2)
        );
        instance.onJobResult(
            testServiceId,
            1, // JOB_EXEC
            900,
            operator2,
            bytes(""),
            encodeJsonOutputs("{}")
        );
    }
}
