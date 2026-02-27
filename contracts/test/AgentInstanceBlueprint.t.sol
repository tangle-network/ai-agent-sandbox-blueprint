// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./helpers/InstanceSetup.sol";

contract AgentInstanceBlueprintTest is InstanceBlueprintTestSetup {
    using stdStorage for StdStorage;

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
        emit AgentSandboxBlueprint.OperatorProvisioned(
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

        // Use literal job ID (5 = PROVISION) to avoid staticcall consuming expectRevert
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.AlreadyProvisioned.selector, testServiceId, operator1)
        );
        instance.onJobResult(
            testServiceId,
            5, // JOB_PROVISION
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

        uint64 callId = 500;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));

        vm.expectEmit(true, true, false, false);
        emit AgentSandboxBlueprint.OperatorDeprovisioned(testServiceId, operator1);

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

        // Use literal job ID (6 = DEPROVISION)
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.NotProvisioned.selector, testServiceId, operator1)
        );
        instance.onJobResult(
            testServiceId,
            6, // JOB_DEPROVISION
            callId,
            operator1,
            bytes(""),
            encodeJsonOutputs("{}")
        );
    }

    function test_deprovisionSwapAndPop() public {
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

        (address[] memory ops, string[] memory urls) = instance.getOperatorEndpoints(testServiceId);
        assertEq(ops.length, 2);
        assertEq(urls.length, 2);
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

        assertEq(ops[0], operator1);
        assertEq(ops[1], operator2);
        assertEq(urls[0], "http://op1:8080");
        assertEq(urls[1], "http://op2:9090");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PRICING
    // ═══════════════════════════════════════════════════════════════════════════

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
        assertEq(ids.length, 7);
        assertEq(ids[0], 0); // SANDBOX_CREATE
        assertEq(ids[5], 5); // PROVISION
        assertEq(ids[6], 6); // DEPROVISION

        assertTrue(instance.supportsJob(0));
        assertTrue(instance.supportsJob(6));
        assertFalse(instance.supportsJob(7));

        assertEq(instance.jobCount(), 7);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // FULL LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    function test_fullProvisionDeprovisionCycle() public {
        // 1. Provision
        _provisionOperator(operator1);
        assertTrue(instance.isProvisioned(testServiceId));
        assertEq(instance.getOperatorCount(testServiceId), 1);

        // 2. Deprovision
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
        emit AgentSandboxBlueprint.ServiceTerminationReceived(testServiceId, blueprintOwner);

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

        // 2. Termination signal from Tangle
        vm.prank(tangleCore);
        instance.onServiceTermination(testServiceId, blueprintOwner);

        // 3. Deprovision both operators
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

        // 4. Verify cleanup
        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertFalse(instance.isProvisioned(testServiceId));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SERVICE CONFIG STORAGE
    // ═══════════════════════════════════════════════════════════════════════════

    function test_serviceConfigStoredOnInitialized() public {
        // 1. Submit service request with config
        bytes memory config = abi.encode("test-sandbox-config");
        address[] memory operators = new address[](1);
        operators[0] = operator1;

        vm.prank(tangleCore);
        instance.onRequest(1, blueprintOwner, operators, config, 0, address(0), 0);

        // 2. Service initialized — config should move to serviceConfig
        address[] memory permittedCallers = new address[](0);
        vm.prank(tangleCore);
        instance.onServiceInitialized(testBlueprintId, 1, testServiceId, blueprintOwner, permittedCallers, 0);

        // 3. Verify config stored by serviceId
        bytes memory stored = instance.getServiceConfig(testServiceId);
        assertEq(stored, config);
    }

    function test_doubleServiceInitializedReverts() public {
        // First initialization
        bytes memory config = abi.encode("config-data");
        address[] memory operators = new address[](1);
        operators[0] = operator1;

        vm.prank(tangleCore);
        instance.onRequest(1, blueprintOwner, operators, config, 0, address(0), 0);

        address[] memory permittedCallers = new address[](0);
        vm.prank(tangleCore);
        instance.onServiceInitialized(testBlueprintId, 1, testServiceId, blueprintOwner, permittedCallers, 0);

        // Second initialization for the same serviceId should revert
        vm.prank(tangleCore);
        instance.onRequest(2, blueprintOwner, operators, config, 0, address(0), 0);

        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.ServiceAlreadyInitialized.selector, testServiceId));
        instance.onServiceInitialized(testBlueprintId, 2, testServiceId, blueprintOwner, permittedCallers, 0);
    }

    function test_serviceConfigEmitsEvent() public {
        bytes memory config = abi.encode("config-data");
        address[] memory operators = new address[](1);
        operators[0] = operator1;

        vm.prank(tangleCore);
        instance.onRequest(1, blueprintOwner, operators, config, 0, address(0), 0);

        vm.expectEmit(true, true, false, false);
        emit AgentSandboxBlueprint.ServiceConfigStored(testServiceId, 1);

        address[] memory permittedCallers = new address[](0);
        vm.prank(tangleCore);
        instance.onServiceInitialized(testBlueprintId, 1, testServiceId, blueprintOwner, permittedCallers, 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SECURITY: OPERATOR ARRAY BOUNDS (H5b)
    // ═══════════════════════════════════════════════════════════════════════════

    // ═══════════════════════════════════════════════════════════════════════════
    // SECURITY: SERVICE REQUEST VALIDATED EVENT (M7 — instance mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onRequestEmitsServiceRequestValidatedInstanceMode() public {
        address[] memory operators = new address[](1);
        operators[0] = operator1;

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.ServiceRequestValidated(1, blueprintOwner, 1);

        vm.prank(tangleCore);
        instance.onRequest(1, blueprintOwner, operators, bytes(""), 0, address(0), 0);
    }

    function test_onRequestEmitsServiceRequestValidatedWithConfig() public {
        bytes memory config = abi.encode("config-data");
        address[] memory operators = new address[](2);
        operators[0] = operator1;
        operators[1] = operator2;

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.ServiceRequestValidated(5, blueprintOwner, 2);

        vm.prank(tangleCore);
        instance.onRequest(5, blueprintOwner, operators, config, 0, address(0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MODE ENFORCEMENT — INSTANCE MODE REJECTS CLOUD JOBS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_instanceModeRejectsCloudModeJobCalls() public {
        // Cloud mode jobs 0-4 should all revert with CloudModeOnly
        for (uint8 jobId = 0; jobId <= 4; jobId++) {
            vm.prank(tangleCore);
            vm.expectRevert(AgentSandboxBlueprint.CloudModeOnly.selector);
            instance.onJobCall(testServiceId, jobId, uint64(2000 + jobId), bytes(""));
        }
    }

    function test_instanceModeRejectsCloudModeJobResults() public {
        for (uint8 jobId = 0; jobId <= 4; jobId++) {
            vm.prank(tangleCore);
            vm.expectRevert(AgentSandboxBlueprint.CloudModeOnly.selector);
            instance.onJobResult(testServiceId, jobId, uint64(2010 + jobId), operator1, bytes(""), bytes(""));
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // UNKNOWN JOB ID REVERTS (INSTANCE MODE)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_unknownJobIdRevertsOnJobCallInstanceMode() public {
        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, 7));
        instance.onJobCall(testServiceId, 7, 3000, bytes(""));
    }

    function test_unknownJobIdRevertsOnJobResultInstanceMode() public {
        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, 7));
        instance.onJobResult(testServiceId, 7, 3001, operator1, bytes(""), bytes(""));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // totalProvisionedOperators COUNTER
    // ═══════════════════════════════════════════════════════════════════════════

    function test_totalProvisionedOperatorsIncrementsOnProvision() public {
        assertEq(instance.totalProvisionedOperators(), 0);

        _provisionOperator(operator1);
        assertEq(instance.totalProvisionedOperators(), 1);

        _provisionOperator(operator2);
        assertEq(instance.totalProvisionedOperators(), 2);
    }

    function test_totalProvisionedOperatorsDecrementsOnDeprovision() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);
        assertEq(instance.totalProvisionedOperators(), 2);

        // Deprovision operator1
        uint64 callId = 3010;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId, operator1, bytes(""), encodeJsonOutputs("{}")
        );

        assertEq(instance.totalProvisionedOperators(), 1);

        // Deprovision operator2
        callId = 3011;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId, operator2, bytes(""), encodeJsonOutputs("{}")
        );

        assertEq(instance.totalProvisionedOperators(), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // setInstanceMode / setTeeRequired GUARD WITH PROVISIONED OPERATORS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_setInstanceModeRevertsWithProvisionedOperators() public {
        _provisionOperator(operator1);
        assertEq(instance.totalProvisionedOperators(), 1);

        vm.prank(blueprintOwner);
        vm.expectRevert(AgentSandboxBlueprint.CannotChangeWithActiveResources.selector);
        instance.setInstanceMode(false);
    }

    function test_setTeeRequiredRevertsWithProvisionedOperators() public {
        _provisionOperator(operator1);
        assertEq(instance.totalProvisionedOperators(), 1);

        vm.prank(blueprintOwner);
        vm.expectRevert(AgentSandboxBlueprint.CannotChangeWithActiveResources.selector);
        instance.setTeeRequired(true);
    }

    function test_setInstanceModeSucceedsAfterFullDeprovision() public {
        _provisionOperator(operator1);
        assertEq(instance.totalProvisionedOperators(), 1);

        // Deprovision
        uint64 callId = 3020;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId, operator1, bytes(""), encodeJsonOutputs("{}")
        );

        assertEq(instance.totalProvisionedOperators(), 0);

        // Now mode change should succeed
        vm.prank(blueprintOwner);
        instance.setInstanceMode(false);
        assertFalse(instance.instanceMode());
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR UNREGISTER / LEAVE TESTS (instance mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onUnregisterSucceedsWithNoProvisions() public {
        // No active sandboxes in instance mode (counter stays 0)
        vm.prank(tangleCore);
        instance.onUnregister(operator1);
    }

    function test_onOperatorLeftRevertsWithActiveProvisions() public {
        _provisionOperator(operator1);

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.CannotLeaveWithActiveResources.selector);
        instance.onOperatorLeft(testServiceId, operator1);
    }

    function test_onOperatorLeftSucceedsWithNoProvisions() public {
        vm.prank(tangleCore);
        instance.onOperatorLeft(testServiceId, operator1);
    }

    function test_onOperatorLeftSucceedsAfterDeprovision() public {
        _provisionOperator(operator1);

        // Deprovision
        uint64 callId = 4050;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId, operator1, bytes(""), encodeJsonOutputs("{}")
        );

        assertFalse(instance.isOperatorProvisioned(testServiceId, operator1));

        // Now leaving should succeed
        vm.prank(tangleCore);
        instance.onOperatorLeft(testServiceId, operator1);
    }

    function test_canLeaveReturnsFalseWithActiveProvisions() public {
        _provisionOperator(operator1);
        assertFalse(instance.canLeave(testServiceId, operator1));
    }

    function test_canLeaveReturnsTrueWithNoProvisions() public {
        assertTrue(instance.canLeave(testServiceId, operator1));
    }

    function test_canLeaveReturnsTrueAfterDeprovision() public {
        _provisionOperator(operator1);
        assertFalse(instance.canLeave(testServiceId, operator1));

        // Deprovision
        uint64 callId = 4060;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId, instance.JOB_DEPROVISION(), callId, operator1, bytes(""), encodeJsonOutputs("{}")
        );

        assertTrue(instance.canLeave(testServiceId, operator1));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SAFE DECREMENT — DEPROVISION SUCCEEDS EVEN IF COUNTER IS ALREADY 0
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Verifies that _handleDeprovisionResult does not revert if
    ///         instanceOperatorCount or totalProvisionedOperators is somehow 0.
    ///         Uses stdstore to force the counters to 0 after provisioning,
    ///         then deprovisions. Without the safe decrement pattern this would
    ///         underflow and revert under Solidity 0.8 checked arithmetic.
    function test_deprovisionSucceedsWhenCounterAlreadyZero() public {
        _provisionOperator(operator1);

        // Sanity: counters should be 1
        assertEq(instance.getOperatorCount(testServiceId), 1);
        assertEq(instance.totalProvisionedOperators(), 1);

        // Force instanceOperatorCount[testServiceId] to 0
        stdstore
            .target(address(instance))
            .sig("instanceOperatorCount(uint64)")
            .with_key(uint256(testServiceId))
            .checked_write(uint256(0));

        // Force totalProvisionedOperators to 0
        stdstore
            .target(address(instance))
            .sig("totalProvisionedOperators()")
            .checked_write(uint256(0));

        // Verify forced to 0
        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertEq(instance.totalProvisionedOperators(), 0);

        // Deprovision should succeed (no underflow revert)
        uint64 callId = 7000;
        simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_DEPROVISION(),
            callId,
            operator1,
            bytes(""),
            encodeJsonOutputs("{}")
        );

        // Counters remain 0 (clamped, not underflowed)
        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertEq(instance.totalProvisionedOperators(), 0);
        assertFalse(instance.isOperatorProvisioned(testServiceId, operator1));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REVERT PATH: ADMIN FUNCTIONS REVERT FOR NON-OWNER
    // ═══════════════════════════════════════════════════════════════════════════

    function test_setDefaultMaxCapacityRevertsForNonOwnerInstanceMode() public {
        vm.prank(operator1);
        vm.expectRevert();
        instance.setDefaultMaxCapacity(500);
    }

    function test_setOperatorCapacityRevertsForNonOwnerInstanceMode() public {
        vm.prank(operator1);
        vm.expectRevert();
        instance.setOperatorCapacity(operator1, 200);
    }

    function test_setInstanceModeRevertsForNonOwner() public {
        vm.prank(operator1);
        vm.expectRevert();
        instance.setInstanceMode(false);
    }

    function test_setTeeRequiredRevertsForNonOwner() public {
        vm.prank(operator1);
        vm.expectRevert();
        instance.setTeeRequired(true);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REVERT PATH: PROVISION WITH EMPTY SANDBOX ID
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onRequestRevertsWithZeroOperatorsInstanceMode() public {
        address[] memory operators = new address[](0);

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.ZeroOperatorsInRequest.selector);
        instance.onRequest(1, blueprintOwner, operators, bytes(""), 0, address(0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // FUZZ TESTS: PROVISION/DEPROVISION
    // ═══════════════════════════════════════════════════════════════════════════

    function testFuzz_provisionWithArbitrarySandboxId(string calldata sandboxId) public {
        // Skip empty strings — they are valid in instance mode (no on-chain length check for provision)
        // Provision encodes sandbox ID in outputs and stores sidecar URL
        uint64 callId = 9000;
        simulateJobCall(testServiceId, instance.JOB_PROVISION(), callId, bytes(""));
        simulateJobResult(
            testServiceId,
            instance.JOB_PROVISION(),
            callId,
            operator1,
            bytes(""),
            abi.encode(sandboxId, "http://sidecar:8080", uint32(2222), "")
        );

        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(instance.getOperatorCount(testServiceId), 1);
        assertEq(instance.operatorSidecarUrl(testServiceId, operator1), "http://sidecar:8080");
    }

    function testFuzz_provisionDeprovisionSequence(uint8 numOps) public {
        // Bound to a reasonable range: 1 to 10 operators
        vm.assume(numOps >= 1 && numOps <= 10);

        address[] memory ops = new address[](numOps);

        // Provision all operators
        for (uint8 i = 0; i < numOps; i++) {
            address op = address(uint160(0x3000 + i));
            ops[i] = op;
            uint64 callId = uint64(10000 + i);
            simulateJobCall(testServiceId, instance.JOB_PROVISION(), callId, bytes(""));
            simulateJobResult(
                testServiceId,
                instance.JOB_PROVISION(),
                callId,
                op,
                bytes(""),
                abi.encode(
                    string(abi.encodePacked("sb-fuzz-", vm.toString(i))),
                    string(abi.encodePacked("http://op", vm.toString(i), ":8080")),
                    uint32(2222 + i),
                    ""
                )
            );
            assertTrue(instance.isOperatorProvisioned(testServiceId, op));
        }

        assertEq(instance.getOperatorCount(testServiceId), uint32(numOps));
        assertEq(instance.totalProvisionedOperators(), uint256(numOps));

        // Deprovision all operators in reverse order
        for (uint256 i = numOps; i > 0; i--) {
            address op = ops[i - 1];
            uint64 callId = uint64(20000 + i);
            simulateJobCall(testServiceId, instance.JOB_DEPROVISION(), callId, bytes(""));
            simulateJobResult(
                testServiceId,
                instance.JOB_DEPROVISION(),
                callId,
                op,
                bytes(""),
                encodeJsonOutputs("{}")
            );
            assertFalse(instance.isOperatorProvisioned(testServiceId, op));
        }

        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertEq(instance.totalProvisionedOperators(), 0);
        assertFalse(instance.isProvisioned(testServiceId));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SECURITY: DOUBLE INITIALIZATION WITH DIFFERENT OWNERS (P5)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_doubleServiceInitializedDifferentOwnerReverts() public {
        bytes memory config = abi.encode("config");
        address[] memory operators = new address[](1);
        operators[0] = operator1;

        // First init with blueprintOwner
        vm.prank(tangleCore);
        instance.onRequest(1, blueprintOwner, operators, config, 0, address(0), 0);

        address[] memory permittedCallers = new address[](0);
        vm.prank(tangleCore);
        instance.onServiceInitialized(testBlueprintId, 1, testServiceId, blueprintOwner, permittedCallers, 0);

        assertEq(instance.serviceOwner(testServiceId), blueprintOwner);

        // Second init with different owner — should revert, not overwrite
        address attacker = address(0xDEAD);
        vm.prank(tangleCore);
        instance.onRequest(2, attacker, operators, config, 0, address(0), 0);

        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.ServiceAlreadyInitialized.selector, testServiceId));
        instance.onServiceInitialized(testBlueprintId, 2, testServiceId, attacker, permittedCallers, 0);

        // Verify owner unchanged
        assertEq(instance.serviceOwner(testServiceId), blueprintOwner);
    }
}
