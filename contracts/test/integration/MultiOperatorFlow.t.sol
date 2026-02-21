// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "../helpers/Setup.sol";

contract MultiOperatorFlowTest is BlueprintTestSetup {
    // Additional operators beyond the base 3 from BlueprintTestSetup
    address public operator4 = address(0x1004);
    address public operator5 = address(0x1005);
    address public operator6 = address(0x1006);
    address public operator7 = address(0x1007);
    address public operator8 = address(0x1008);

    // Custom event for gas profiling test
    event GasUsed(string operation, uint256 gasUsed);

    function setUp() public override {
        super.setUp();
        registerOperator(operator4, 10);
        registerOperator(operator5, 10);
        registerOperator(operator6, 10);
        registerOperator(operator7, 10);
        registerOperator(operator8, 10);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // HELPER: Create sandbox forcing assignment to a specific operator.
    // ═══════════════════════════════════════════════════════════════════════════

    function _allOperators() internal view returns (address[] memory) {
        address[] memory ops = new address[](8);
        ops[0] = operator1;
        ops[1] = operator2;
        ops[2] = operator3;
        ops[3] = operator4;
        ops[4] = operator5;
        ops[5] = operator6;
        ops[6] = operator7;
        ops[7] = operator8;
        return ops;
    }

    function _createSandboxOnOperator(
        uint64 serviceId,
        uint64 callId,
        address targetOperator,
        string memory sandboxId
    ) internal {
        address[] memory ops = _allOperators();

        for (uint256 i = 0; i < ops.length; i++) {
            if (ops[i] != targetOperator) {
                mockDelegation.setActive(ops[i], false);
            }
        }

        simulateJobCall(serviceId, blueprint.JOB_SANDBOX_CREATE(), callId, encodeSandboxCreateInputs());

        for (uint256 i = 0; i < ops.length; i++) {
            if (ops[i] != targetOperator) {
                mockDelegation.setActive(ops[i], true);
            }
        }

        simulateJobResult(
            serviceId,
            blueprint.JOB_SANDBOX_CREATE(),
            callId,
            targetOperator,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs(sandboxId, "{}")
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TEST 1: Full multi-operator lifecycle (create + delete)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_fullMultiOperatorLifecycle() public {
        vm.startPrank(blueprintOwner);
        blueprint.setOperatorCapacity(operator4, 3);
        blueprint.setOperatorCapacity(operator5, 10);
        blueprint.setOperatorCapacity(operator6, 50);
        blueprint.setOperatorCapacity(operator7, 2);
        blueprint.setOperatorCapacity(operator8, 100);
        vm.stopPrank();

        // Create a sandbox on each operator
        _createSandboxOnOperator(1, 1000, operator4, "sb-op4");
        assertEq(blueprint.getSandboxOperator("sb-op4"), operator4);

        _createSandboxOnOperator(1, 1001, operator5, "sb-op5");
        assertEq(blueprint.getSandboxOperator("sb-op5"), operator5);

        _createSandboxOnOperator(1, 1002, operator6, "sb-op6");
        assertEq(blueprint.getSandboxOperator("sb-op6"), operator6);

        _createSandboxOnOperator(1, 1003, operator7, "sb-op7");
        assertEq(blueprint.getSandboxOperator("sb-op7"), operator7);

        _createSandboxOnOperator(1, 1004, operator8, "sb-op8");
        assertEq(blueprint.getSandboxOperator("sb-op8"), operator8);

        assertEq(blueprint.totalActiveSandboxes(), 5);

        // Verify per-operator load
        (uint32 active4,) = blueprint.getOperatorLoad(operator4);
        (uint32 active5,) = blueprint.getOperatorLoad(operator5);
        (uint32 active6,) = blueprint.getOperatorLoad(operator6);
        (uint32 active7,) = blueprint.getOperatorLoad(operator7);
        (uint32 active8,) = blueprint.getOperatorLoad(operator8);
        assertEq(active4, 1);
        assertEq(active5, 1);
        assertEq(active6, 1);
        assertEq(active7, 1);
        assertEq(active8, 1);

        // Delete sandboxes on operator4 and operator7
        simulateJobCall(1, blueprint.JOB_SANDBOX_DELETE(), 1020, encodeSandboxIdInputs("sb-op4"));
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_DELETE(), 1020, operator4,
            encodeSandboxIdInputs("sb-op4"), encodeJsonOutputs("{\"deleted\":true}")
        );

        simulateJobCall(1, blueprint.JOB_SANDBOX_DELETE(), 1021, encodeSandboxIdInputs("sb-op7"));
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_DELETE(), 1021, operator7,
            encodeSandboxIdInputs("sb-op7"), encodeJsonOutputs("{\"deleted\":true}")
        );

        assertEq(blueprint.totalActiveSandboxes(), 3);
        assertFalse(blueprint.isSandboxActive("sb-op4"));
        assertFalse(blueprint.isSandboxActive("sb-op7"));

        // Verify capacity recovered
        (uint32 active4After,) = blueprint.getOperatorLoad(operator4);
        assertEq(active4After, 0);
        (uint32 active7After,) = blueprint.getOperatorLoad(operator7);
        assertEq(active7After, 0);

        // Remaining sandboxes still routed correctly
        assertEq(blueprint.getSandboxOperator("sb-op5"), operator5);
        assertEq(blueprint.getSandboxOperator("sb-op6"), operator6);
        assertEq(blueprint.getSandboxOperator("sb-op8"), operator8);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TEST 2: Capacity exhaustion and recovery
    // ═══════════════════════════════════════════════════════════════════════════

    function test_capacityExhaustion() public {
        mockDelegation.setActive(operator1, false);
        mockDelegation.setActive(operator2, false);
        mockDelegation.setActive(operator3, false);
        mockDelegation.setActive(operator5, false);
        mockDelegation.setActive(operator6, false);
        mockDelegation.setActive(operator7, false);
        mockDelegation.setActive(operator8, false);

        vm.prank(blueprintOwner);
        blueprint.setOperatorCapacity(operator4, 2);

        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 2000, encodeSandboxCreateInputs());
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_CREATE(), 2000, operator4,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("cap-1", "{}")
        );

        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 2001, encodeSandboxCreateInputs());
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_CREATE(), 2001, operator4,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("cap-2", "{}")
        );

        (uint32 active, uint32 max) = blueprint.getOperatorLoad(operator4);
        assertEq(active, 2);
        assertEq(max, 2);

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.NoAvailableCapacity.selector);
        blueprint.onJobCall(1, 0, 2002, encodeSandboxCreateInputs());

        // Delete one sandbox to free capacity
        simulateJobCall(1, blueprint.JOB_SANDBOX_DELETE(), 2003, encodeSandboxIdInputs("cap-1"));
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_DELETE(), 2003, operator4,
            encodeSandboxIdInputs("cap-1"), encodeJsonOutputs("{\"deleted\":true}")
        );

        (uint32 activeAfter,) = blueprint.getOperatorLoad(operator4);
        assertEq(activeAfter, 1);

        // Now a new create should succeed
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 2004, encodeSandboxCreateInputs());
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_CREATE(), 2004, operator4,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("cap-3", "{}")
        );
        (uint32 activeFinal,) = blueprint.getOperatorLoad(operator4);
        assertEq(activeFinal, 2);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TEST 3: Batch distribution across equal-capacity operators
    // ═══════════════════════════════════════════════════════════════════════════

    function test_batchDistribution() public {
        mockDelegation.setActive(operator1, false);
        mockDelegation.setActive(operator2, false);
        mockDelegation.setActive(operator3, false);
        mockDelegation.setActive(operator7, false);
        mockDelegation.setActive(operator8, false);

        uint256 count4 = 0;
        uint256 count5 = 0;
        uint256 count6 = 0;

        for (uint256 i = 0; i < 30; i++) {
            vm.prevrandao(bytes32(uint256(keccak256(abi.encode("batch", i)))));
            vm.recordLogs();
            simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), uint64(3000 + i), encodeSandboxCreateInputs());

            Vm.Log[] memory logs = vm.getRecordedLogs();
            address assigned;
            for (uint256 j = 0; j < logs.length; j++) {
                if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                    assigned = address(uint160(uint256(logs[j].topics[3])));
                }
            }

            string memory sid = string(abi.encodePacked("batch-", vm.toString(i)));
            simulateJobResult(
                1, blueprint.JOB_SANDBOX_CREATE(), uint64(3000 + i), assigned,
                encodeSandboxCreateInputs(),
                encodeSandboxCreateOutputs(sid, "{}")
            );

            if (assigned == operator4) count4++;
            else if (assigned == operator5) count5++;
            else if (assigned == operator6) count6++;
        }

        assertGt(count4, 3, "operator4 should receive meaningful share");
        assertGt(count5, 3, "operator5 should receive meaningful share");
        assertGt(count6, 3, "operator6 should receive meaningful share");
        assertEq(count4 + count5 + count6, 30, "all 30 sandboxes must be accounted for");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TEST 4: Gas profiling for create and delete
    // ═══════════════════════════════════════════════════════════════════════════

    function test_gasProfile_createAndDelete() public {
        mockDelegation.setActive(operator1, false);
        mockDelegation.setActive(operator2, false);
        mockDelegation.setActive(operator3, false);
        mockDelegation.setActive(operator5, false);
        mockDelegation.setActive(operator6, false);
        mockDelegation.setActive(operator7, false);
        mockDelegation.setActive(operator8, false);

        vm.prank(blueprintOwner);
        blueprint.setOperatorCapacity(operator4, 100);

        uint256 gasBefore = gasleft();
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 4000, encodeSandboxCreateInputs());
        uint256 gasCreateCall = gasBefore - gasleft();
        emit GasUsed("onJobCall_CREATE", gasCreateCall);

        gasBefore = gasleft();
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_CREATE(), 4000, operator4,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("gas-sb", "{}")
        );
        uint256 gasCreateResult = gasBefore - gasleft();
        emit GasUsed("onJobResult_CREATE", gasCreateResult);

        gasBefore = gasleft();
        simulateJobCall(1, blueprint.JOB_SANDBOX_DELETE(), 4001, encodeSandboxIdInputs("gas-sb"));
        uint256 gasDeleteCall = gasBefore - gasleft();
        emit GasUsed("onJobCall_DELETE", gasDeleteCall);

        gasBefore = gasleft();
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_DELETE(), 4001, operator4,
            encodeSandboxIdInputs("gas-sb"),
            encodeJsonOutputs("{\"deleted\":true}")
        );
        uint256 gasDeleteResult = gasBefore - gasleft();
        emit GasUsed("onJobResult_DELETE", gasDeleteResult);

        assertLt(gasCreateCall, 500_000, "CREATE call gas too high");
        assertLt(gasCreateResult, 500_000, "CREATE result gas too high");
        assertLt(gasDeleteCall, 500_000, "DELETE call gas too high");
        assertLt(gasDeleteResult, 500_000, "DELETE result gas too high");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TEST 5: Operator deregistration mid-lifecycle
    // ═══════════════════════════════════════════════════════════════════════════

    function test_operatorDeregistrationMidLifecycle() public {
        _createSandboxOnOperator(1, 5000, operator4, "dereg-sb");
        assertEq(blueprint.getSandboxOperator("dereg-sb"), operator4);
        assertTrue(blueprint.isSandboxActive("dereg-sb"));

        // Deactivate operator4
        mockDelegation.setActive(operator4, false);

        // New creates should NOT route to inactive operator4
        mockDelegation.setActive(operator1, false);
        mockDelegation.setActive(operator2, false);
        mockDelegation.setActive(operator3, false);
        mockDelegation.setActive(operator6, false);
        mockDelegation.setActive(operator7, false);
        mockDelegation.setActive(operator8, false);

        vm.recordLogs();
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 5003, encodeSandboxCreateInputs());

        Vm.Log[] memory logs = vm.getRecordedLogs();
        address assignedNew;
        for (uint256 j = 0; j < logs.length; j++) {
            if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                assignedNew = address(uint160(uint256(logs[j].topics[3])));
            }
        }
        assertEq(assignedNew, operator5, "inactive operator4 must not be selected for new creates");

        // Reactivate for cleanup
        mockDelegation.setActive(operator1, true);
        mockDelegation.setActive(operator2, true);
        mockDelegation.setActive(operator3, true);
        mockDelegation.setActive(operator4, true);
        mockDelegation.setActive(operator6, true);
        mockDelegation.setActive(operator7, true);
        mockDelegation.setActive(operator8, true);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TEST 6: Concurrent creates with same callId across different serviceIds
    // ═══════════════════════════════════════════════════════════════════════════

    function test_concurrentCreatesSameCallId() public {
        uint64 serviceA = 1;
        uint64 serviceB = 2;
        uint64 sharedCallId = 999;

        _createSandboxOnOperator(serviceA, sharedCallId, operator4, "svc1-sb");
        _createSandboxOnOperator(serviceB, sharedCallId, operator5, "svc2-sb");

        assertEq(blueprint.getSandboxOperator("svc1-sb"), operator4);
        assertEq(blueprint.getSandboxOperator("svc2-sb"), operator5);
        assertTrue(blueprint.isSandboxActive("svc1-sb"));
        assertTrue(blueprint.isSandboxActive("svc2-sb"));
        assertEq(blueprint.totalActiveSandboxes(), 2);

        simulateJobCall(serviceA, blueprint.JOB_SANDBOX_DELETE(), 6001, encodeSandboxIdInputs("svc1-sb"));
        simulateJobResult(
            serviceA, blueprint.JOB_SANDBOX_DELETE(), 6001, operator4,
            encodeSandboxIdInputs("svc1-sb"), encodeJsonOutputs("{\"deleted\":true}")
        );

        assertFalse(blueprint.isSandboxActive("svc1-sb"));
        assertTrue(blueprint.isSandboxActive("svc2-sb"));
        assertEq(blueprint.getSandboxOperator("svc2-sb"), operator5);
        assertEq(blueprint.totalActiveSandboxes(), 1);
    }
}
