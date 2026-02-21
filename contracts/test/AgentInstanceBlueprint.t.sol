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

    function test_getDefaultJobRates() public view {
        uint256 baseRate = 1 ether;
        (uint8[] memory jobs, uint256[] memory rates) = instance.getDefaultJobRates(baseRate);

        assertEq(jobs.length, 7);
        assertEq(rates.length, 7);

        // Verify each rate = baseRate * multiplier
        assertEq(rates[0], baseRate * 50);  // SANDBOX_CREATE
        assertEq(rates[1], baseRate * 1);   // SANDBOX_DELETE
        assertEq(rates[2], baseRate * 2);   // WORKFLOW_CREATE
        assertEq(rates[3], baseRate * 5);   // WORKFLOW_TRIGGER
        assertEq(rates[4], baseRate * 1);   // WORKFLOW_CANCEL
        assertEq(rates[5], baseRate * 50);  // PROVISION
        assertEq(rates[6], baseRate * 1);   // DEPROVISION
    }

    function test_getJobPriceMultiplier() public view {
        assertEq(instance.getJobPriceMultiplier(instance.JOB_PROVISION()), 50);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_DEPROVISION()), 1);
        assertEq(instance.getJobPriceMultiplier(instance.JOB_SANDBOX_CREATE()), 50);
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
        assertEq(ids.length, 7);
        assertEq(ids[0], 0); // SANDBOX_CREATE
        assertEq(ids[5], 5); // PROVISION
        assertEq(ids[6], 6); // DEPROVISION

        assertTrue(instance.supportsJob(0));
        assertTrue(instance.supportsJob(6));
        assertFalse(instance.supportsJob(7));

        assertEq(instance.jobCount(), 7);
    }

    function test_blueprintMetadata() public view {
        assertEq(instance.BLUEPRINT_NAME(), "ai-agent-sandbox-blueprint");
        assertEq(instance.BLUEPRINT_VERSION(), "0.4.0");
    }

    function test_instanceModeEnabled() public view {
        assertTrue(instance.instanceMode());
        assertFalse(instance.teeRequired());
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
}
