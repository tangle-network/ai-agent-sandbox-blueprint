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
        setServiceOperator(testServiceId, operator1, true);

        vm.expectEmit(true, true, false, true);
        emit AgentSandboxBlueprint.OperatorProvisioned(
            testServiceId,
            operator1,
            string(abi.encodePacked("sb-", vm.toString(operator1))),
            "http://sidecar:8080"
        );

        vm.prank(operator1);
        instance.reportProvisioned(
            testServiceId,
            string(abi.encodePacked("sb-", vm.toString(operator1))),
            "http://sidecar:8080",
            2222,
            ""
        );
    }

    function test_provisionAlreadyProvisionedReverts() public {
        _provisionOperator(operator1);

        setServiceOperator(testServiceId, operator1, true);
        vm.prank(operator1);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.AlreadyProvisioned.selector, testServiceId, operator1)
        );
        instance.reportProvisioned(testServiceId, "sb-dup", "http://dup:8080", 2222, "");
    }

    function test_provisionWithAttestationStoresHash() public {
        string memory attestation = '{"tee":"nitro","quote":"deadbeef"}';
        _provisionOperatorFull(operator1, "http://sidecar:8080", 2222, attestation);

        bytes32 expectedHash = keccak256(bytes(attestation));
        assertEq(instance.getAttestationHash(testServiceId, operator1), expectedHash);
    }

    function test_reportProvisionedByActiveServiceOperator() public {
        setServiceOperator(testServiceId, operator1, true);

        vm.prank(operator1);
        instance.reportProvisioned(testServiceId, "sb-r1", "http://report-op1:8080", 2222, "");

        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(instance.operatorSidecarUrl(testServiceId, operator1), "http://report-op1:8080");
        assertEq(instance.getOperatorCount(testServiceId), 1);
    }

    function test_reportProvisionedNonServiceOperatorReverts() public {
        vm.prank(operator1);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.OperatorNotInService.selector, testServiceId, operator1)
        );
        instance.reportProvisioned(testServiceId, "sb-r1", "http://report-op1:8080", 2222, "");
    }

    function test_reportDeprovisionedByActiveServiceOperator() public {
        setServiceOperator(testServiceId, operator1, true);

        vm.prank(operator1);
        instance.reportProvisioned(testServiceId, "sb-r1", "http://report-op1:8080", 2222, "");
        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));

        vm.prank(operator1);
        instance.reportDeprovisioned(testServiceId);
        assertFalse(instance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(instance.getOperatorCount(testServiceId), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // DEPROVISION FLOW
    // ═══════════════════════════════════════════════════════════════════════════

    function test_deprovisionCleanRemoval() public {
        _provisionOperator(operator1);
        assertTrue(instance.isOperatorProvisioned(testServiceId, operator1));

        vm.expectEmit(true, true, false, false);
        emit AgentSandboxBlueprint.OperatorDeprovisioned(testServiceId, operator1);

        _deprovisionOperator(operator1);

        assertFalse(instance.isOperatorProvisioned(testServiceId, operator1));
        assertEq(instance.getOperatorCount(testServiceId), 0);
        assertFalse(instance.isProvisioned(testServiceId));
    }

    function test_deprovisionNotProvisionedReverts() public {
        setServiceOperator(testServiceId, operator1, true);
        vm.prank(operator1);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.NotProvisioned.selector, testServiceId, operator1)
        );
        instance.reportDeprovisioned(testServiceId);
    }

    function test_deprovisionSwapAndPop() public {
        _provisionOperator(operator1);
        _provisionOperator(operator2);
        _provisionOperator(operator3);

        assertEq(instance.getOperatorCount(testServiceId), 3);

        // Deprovision the middle one (operator2) — triggers swap-and-pop
        _deprovisionOperator(operator2);

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
        assertEq(ids.length, 5);
        assertEq(ids[0], 0); // SANDBOX_CREATE
        assertEq(ids[4], 4); // WORKFLOW_CANCEL

        assertTrue(instance.supportsJob(0));
        assertTrue(instance.supportsJob(4));
        assertFalse(instance.supportsJob(5));

        assertEq(instance.jobCount(), 5);
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
        _deprovisionOperator(operator1);

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
        _deprovisionOperator(operator1);
        _deprovisionOperator(operator2);

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
    // MODE ENFORCEMENT — INSTANCE MODE REJECTS SANDBOX FLEET JOBS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_instanceModeRejectsCloudModeJobCalls() public {
        // Fleet-only jobs 0-1 should revert with CloudModeOnly
        for (uint8 jobId = 0; jobId <= 1; jobId++) {
            vm.prank(tangleCore);
            vm.expectRevert(AgentSandboxBlueprint.CloudModeOnly.selector);
            instance.onJobCall(testServiceId, jobId, uint64(2000 + jobId), bytes(""));
        }
    }

    function test_instanceModeRejectsCloudModeJobResults() public {
        for (uint8 jobId = 0; jobId <= 1; jobId++) {
            vm.prank(tangleCore);
            vm.expectRevert(AgentSandboxBlueprint.CloudModeOnly.selector);
            instance.onJobResult(testServiceId, jobId, uint64(2010 + jobId), operator1, bytes(""), bytes(""));
        }
    }

    function test_instanceModeAllowsWorkflowJobs() public {
        AgentSandboxBlueprint.WorkflowCreateRequest memory req = AgentSandboxBlueprint.WorkflowCreateRequest({
            name: "instance-workflow",
            workflow_json: "{\"prompt\":\"hello\"}",
            trigger_type: "cron",
            trigger_config: "0 * * * * *",
            sandbox_config_json: "{}"
        });

        uint64 createCallId = 2100;
        simulateJobCall(testServiceId, instance.JOB_WORKFLOW_CREATE(), createCallId, abi.encode(req));
        simulateJobResult(
            testServiceId,
            instance.JOB_WORKFLOW_CREATE(),
            createCallId,
            operator1,
            abi.encode(req),
            bytes("")
        );

        AgentSandboxBlueprint.WorkflowConfig memory cfg = instance.getWorkflow(createCallId);
        assertEq(cfg.name, "instance-workflow");
        assertTrue(cfg.active);

        AgentSandboxBlueprint.WorkflowControlRequest memory ctrl = AgentSandboxBlueprint.WorkflowControlRequest({
            workflow_id: createCallId
        });

        uint64 triggerCallId = 2101;
        simulateJobCall(testServiceId, instance.JOB_WORKFLOW_TRIGGER(), triggerCallId, abi.encode(ctrl));
        simulateJobResult(
            testServiceId,
            instance.JOB_WORKFLOW_TRIGGER(),
            triggerCallId,
            operator1,
            abi.encode(ctrl),
            bytes("")
        );
        assertEq(instance.getWorkflow(createCallId).last_triggered_at, uint64(block.timestamp));

        uint64 cancelCallId = 2102;
        simulateJobCall(testServiceId, instance.JOB_WORKFLOW_CANCEL(), cancelCallId, abi.encode(ctrl));
        simulateJobResult(
            testServiceId,
            instance.JOB_WORKFLOW_CANCEL(),
            cancelCallId,
            operator1,
            abi.encode(ctrl),
            bytes("")
        );
        assertFalse(instance.getWorkflow(createCallId).active);
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
        _deprovisionOperator(operator1);

        assertEq(instance.totalProvisionedOperators(), 1);

        // Deprovision operator2
        _deprovisionOperator(operator2);

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
        _deprovisionOperator(operator1);

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
        _deprovisionOperator(operator1);

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
        _deprovisionOperator(operator1);

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
        _deprovisionOperator(operator1);

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
        setServiceOperator(testServiceId, operator1, true);
        vm.prank(operator1);
        instance.reportProvisioned(testServiceId, sandboxId, "http://sidecar:8080", 2222, "");

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
            setServiceOperator(testServiceId, op, true);
            vm.prank(op);
            instance.reportProvisioned(
                testServiceId,
                string(abi.encodePacked("sb-fuzz-", vm.toString(i))),
                string(abi.encodePacked("http://op", vm.toString(i), ":8080")),
                uint32(2222 + i),
                ""
            );
            assertTrue(instance.isOperatorProvisioned(testServiceId, op));
        }

        assertEq(instance.getOperatorCount(testServiceId), uint32(numOps));
        assertEq(instance.totalProvisionedOperators(), uint256(numOps));

        // Deprovision all operators in reverse order
        for (uint256 i = numOps; i > 0; i--) {
            address op = ops[i - 1];
            setServiceOperator(testServiceId, op, true);
            vm.prank(op);
            instance.reportDeprovisioned(testServiceId);
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
