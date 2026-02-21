// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./helpers/Setup.sol";

contract AgentSandboxBlueprintTest is BlueprintTestSetup {

    // ═══════════════════════════════════════════════════════════════════════════
    // REGISTRATION TESTS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_registerWithCapacity() public {
        registerOperator(operator1, 50);
        assertEq(blueprint.operatorMaxCapacity(operator1), 50);
    }

    function test_registerDefaultCapacity() public {
        registerOperator(operator1, 0);
        assertEq(blueprint.operatorMaxCapacity(operator1), 100);
    }

    function test_setOperatorCapacity() public {
        registerOperator(operator1, 50);
        vm.prank(blueprintOwner);
        blueprint.setOperatorCapacity(operator1, 200);
        assertEq(blueprint.operatorMaxCapacity(operator1), 200);
    }

    function test_setDefaultMaxCapacity() public {
        vm.prank(blueprintOwner);
        blueprint.setDefaultMaxCapacity(250);
        assertEq(blueprint.defaultMaxCapacity(), 250);

        registerOperator(operator1, 0);
        assertEq(blueprint.operatorMaxCapacity(operator1), 250);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR ASSIGNMENT (onJobCall + SANDBOX_CREATE)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_assignOperatorOnCreate() public {
        registerOperator(operator1, 10);

        vm.expectEmit(true, true, true, true);
        emit AgentSandboxBlueprint.OperatorAssigned(1, 100, operator1);

        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 100, encodeSandboxCreateInputs());
    }

    function test_assignLeastLoaded() public {
        // Register 3 operators with same capacity
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);
        registerOperator(operator3, 10);

        // Give operator1 and operator2 some load by creating sandboxes
        _createSandbox(1, 1, operator1, "sb-1");
        _createSandbox(1, 2, operator2, "sb-2");
        _createSandbox(1, 3, operator2, "sb-3");

        // operator1: 9 available, operator2: 8 available, operator3: 10 available
        // operator3 has most capacity, should get assigned more often
        uint256 op3Count = 0;
        for (uint256 i = 0; i < 100; i++) {
            vm.prevrandao(bytes32(uint256(i * 7 + 42)));
            vm.recordLogs();
            simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), uint64(1000 + i), encodeSandboxCreateInputs());

            Vm.Log[] memory logs = vm.getRecordedLogs();
            for (uint256 j = 0; j < logs.length; j++) {
                if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                    address assigned = address(uint160(uint256(logs[j].topics[3])));
                    if (assigned == operator3) op3Count++;
                }
            }
        }

        assertGt(op3Count, 25, "operator3 should get significant assignments (~37% expected)");
    }

    function test_weightedDistribution() public {
        // operator1: capacity 100, operator2: capacity 10
        registerOperator(operator1, 100);
        registerOperator(operator2, 10);

        uint256 op1Count = 0;
        for (uint256 i = 0; i < 200; i++) {
            vm.prevrandao(bytes32(uint256(i * 13 + 7)));
            vm.recordLogs();
            simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), uint64(2000 + i), encodeSandboxCreateInputs());

            Vm.Log[] memory logs = vm.getRecordedLogs();
            for (uint256 j = 0; j < logs.length; j++) {
                if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                    address assigned = address(uint160(uint256(logs[j].topics[3])));
                    if (assigned == operator1) op1Count++;
                }
            }
        }

        assertGt(op1Count, 140, "operator1 should get majority of assignments");
        assertLt(op1Count, 200, "operator2 should get some assignments too");
    }

    function test_revertWhenAllFull() public {
        registerOperator(operator1, 1);
        _createSandbox(1, 1, operator1, "sb-full");

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.NoAvailableCapacity.selector);
        blueprint.onJobCall(1, 0, 999, encodeSandboxCreateInputs());
    }

    function test_skipInactiveOperators() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);

        mockDelegation.setActive(operator1, false);

        for (uint256 i = 0; i < 10; i++) {
            vm.prevrandao(bytes32(uint256(i)));
            vm.recordLogs();
            simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), uint64(3000 + i), encodeSandboxCreateInputs());

            Vm.Log[] memory logs = vm.getRecordedLogs();
            for (uint256 j = 0; j < logs.length; j++) {
                if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                    address assigned = address(uint160(uint256(logs[j].topics[3])));
                    assertEq(assigned, operator2, "inactive operator should be skipped");
                }
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SANDBOX CREATE RESULT (onJobResult + SANDBOX_CREATE)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_createResultStoresRouting() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 10, operator1, "sandbox-abc");

        assertEq(blueprint.getSandboxOperator("sandbox-abc"), operator1);
        assertTrue(blueprint.isSandboxActive("sandbox-abc"));
    }

    function test_createResultIncrementsLoad() public {
        registerOperator(operator1, 10);

        (uint32 activeBefore, uint32 maxBefore) = blueprint.getOperatorLoad(operator1);
        assertEq(activeBefore, 0);
        assertEq(maxBefore, 10);

        _createSandbox(1, 20, operator1, "sandbox-inc");

        (uint32 activeAfter, uint32 maxAfter) = blueprint.getOperatorLoad(operator1);
        assertEq(activeAfter, 1);
        assertEq(maxAfter, 10);
        assertEq(blueprint.totalActiveSandboxes(), 1);
    }

    function test_createResultRejectsWrongOperator() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);

        mockDelegation.setActive(operator2, false);
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 30, encodeSandboxCreateInputs());
        mockDelegation.setActive(operator2, true);

        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.OperatorMismatch.selector, operator1, operator2)
        );
        blueprint.onJobResult(
            1, 0, 30, operator2,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("sandbox-wrong", "{}")
        );
    }

    function test_createResultClearsAssignment() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 40, operator1, "sandbox-clear");

        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.OperatorMismatch.selector, address(0), operator1)
        );
        blueprint.onJobResult(
            1, 0, 40, operator1,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("sandbox-clear2", "{}")
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SANDBOX ROUTING (onJobCall for DELETE)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_routeDeleteToCorrectOperator() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 60, operator1, "sandbox-del");

        vm.expectEmit(true, true, true, true);
        emit AgentSandboxBlueprint.OperatorRouted(1, 61, operator1);

        simulateJobCall(1, blueprint.JOB_SANDBOX_DELETE(), 61, encodeSandboxIdInputs("sandbox-del"));
    }

    function test_routeRevertsUnknownSandbox() public {
        bytes32 unknownHash = keccak256(bytes("no-such-sandbox"));
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.SandboxNotFound.selector, unknownHash)
        );
        blueprint.onJobCall(1, 1, 70, encodeSandboxIdInputs("no-such-sandbox"));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SANDBOX DELETE (onJobResult + SANDBOX_DELETE)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_deleteDecrementsLoad() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 80, operator1, "sandbox-dec");

        assertEq(blueprint.totalActiveSandboxes(), 1);
        (uint32 active,) = blueprint.getOperatorLoad(operator1);
        assertEq(active, 1);

        simulateJobResult(
            1,
            blueprint.JOB_SANDBOX_DELETE(),
            81,
            operator1,
            encodeSandboxIdInputs("sandbox-dec"),
            encodeJsonOutputs("{\"deleted\":true}")
        );

        assertEq(blueprint.totalActiveSandboxes(), 0);
        (uint32 activeAfter,) = blueprint.getOperatorLoad(operator1);
        assertEq(activeAfter, 0);
    }

    function test_deleteClearsRouting() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 90, operator1, "sandbox-clr");

        simulateJobResult(
            1,
            blueprint.JOB_SANDBOX_DELETE(),
            91,
            operator1,
            encodeSandboxIdInputs("sandbox-clr"),
            encodeJsonOutputs("{}")
        );

        assertEq(blueprint.getSandboxOperator("sandbox-clr"), address(0));
        assertFalse(blueprint.isSandboxActive("sandbox-clr"));
    }

    function test_deleteRejectsWrongOperator() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);
        _createSandbox(1, 100, operator1, "sandbox-own");

        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.OperatorMismatch.selector, operator1, operator2)
        );
        blueprint.onJobResult(
            1, 1, 101, operator2,
            encodeSandboxIdInputs("sandbox-own"),
            encodeJsonOutputs("{}")
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // FULL LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    function test_fullLifecycle() public {
        registerOperator(operator1, 10);
        string memory sid = "lifecycle-sandbox";

        // 1. Create
        _createSandbox(1, 200, operator1, sid);
        assertTrue(blueprint.isSandboxActive(sid));
        assertEq(blueprint.getSandboxOperator(sid), operator1);
        assertEq(blueprint.totalActiveSandboxes(), 1);

        // 2. Delete
        simulateJobCall(1, blueprint.JOB_SANDBOX_DELETE(), 201, encodeSandboxIdInputs(sid));
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_DELETE(), 201, operator1,
            encodeSandboxIdInputs(sid), encodeJsonOutputs("{\"deleted\":true}")
        );
        assertFalse(blueprint.isSandboxActive(sid));
        assertEq(blueprint.getSandboxOperator(sid), address(0));
        assertEq(blueprint.totalActiveSandboxes(), 0);
    }

    function test_multipleOperatorsMultipleSandboxes() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);
        registerOperator(operator3, 10);

        uint256[3] memory counts;
        address[3] memory ops = [operator1, operator2, operator3];

        for (uint256 i = 0; i < 10; i++) {
            vm.prevrandao(bytes32(uint256(i * 31 + 5)));
            vm.recordLogs();
            simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), uint64(300 + i), encodeSandboxCreateInputs());

            Vm.Log[] memory logs = vm.getRecordedLogs();
            address assigned;
            for (uint256 j = 0; j < logs.length; j++) {
                if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                    assigned = address(uint160(uint256(logs[j].topics[3])));
                }
            }

            string memory sid = string(abi.encodePacked("multi-", vm.toString(i)));
            simulateJobResult(
                1, blueprint.JOB_SANDBOX_CREATE(), uint64(300 + i), assigned,
                encodeSandboxCreateInputs(),
                encodeSandboxCreateOutputs(sid, "{}")
            );

            for (uint256 k = 0; k < 3; k++) {
                if (assigned == ops[k]) counts[k]++;
            }
        }

        assertEq(blueprint.totalActiveSandboxes(), 10);
        assertEq(counts[0] + counts[1] + counts[2], 10, "all sandboxes accounted for");
        uint256 activeOps = 0;
        for (uint256 i = 0; i < 3; i++) {
            if (counts[i] > 0) activeOps++;
        }
        assertGt(activeOps, 1, "at least 2 operators should receive assignments");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_getOperatorLoad() public {
        registerOperator(operator1, 50);
        (uint32 active, uint32 max) = blueprint.getOperatorLoad(operator1);
        assertEq(active, 0);
        assertEq(max, 50);

        _createSandbox(1, 400, operator1, "view-load");
        (active, max) = blueprint.getOperatorLoad(operator1);
        assertEq(active, 1);
        assertEq(max, 50);
    }

    function test_getSandboxOperator() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 410, operator1, "view-op");
        assertEq(blueprint.getSandboxOperator("view-op"), operator1);
        assertEq(blueprint.getSandboxOperator("nonexistent"), address(0));
    }

    function test_getAvailableCapacity() public {
        registerOperator(operator1, 100);
        registerOperator(operator2, 50);

        assertEq(blueprint.getAvailableCapacity(), 150);

        _createSandbox(1, 420, operator1, "cap-1");
        assertEq(blueprint.getAvailableCapacity(), 149);
    }

    function test_isSandboxActive() public {
        registerOperator(operator1, 10);
        assertFalse(blueprint.isSandboxActive("not-here"));

        _createSandbox(1, 430, operator1, "active-check");
        assertTrue(blueprint.isSandboxActive("active-check"));
    }

    function test_getServiceStats() public {
        registerOperator(operator1, 100);
        registerOperator(operator2, 50);

        (uint32 totalSandboxes, uint32 totalCapacity) = blueprint.getServiceStats();
        assertEq(totalSandboxes, 0);
        assertEq(totalCapacity, 150);

        _createSandbox(1, 440, operator1, "stats-1");
        (totalSandboxes, totalCapacity) = blueprint.getServiceStats();
        assertEq(totalSandboxes, 1);
        assertEq(totalCapacity, 150);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // WORKFLOW HOOKS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_workflowCreateStoresConfig() public {
        AgentSandboxBlueprint.WorkflowCreateRequest memory req = AgentSandboxBlueprint.WorkflowCreateRequest({
            name: "test-workflow",
            workflow_json: "{}",
            trigger_type: "cron",
            trigger_config: "0 * * * *",
            sandbox_config_json: "{}"
        });

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.WorkflowStored(500, "cron", "0 * * * *");

        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_CREATE(), 500, operator1,
            abi.encode(req), encodeJsonOutputs("{}")
        );

        AgentSandboxBlueprint.WorkflowConfig memory config = blueprint.getWorkflow(500);
        assertEq(config.name, "test-workflow");
        assertEq(config.trigger_type, "cron");
        assertTrue(config.active);
    }

    function test_workflowTriggerUpdatesTimestamp() public {
        AgentSandboxBlueprint.WorkflowCreateRequest memory req = AgentSandboxBlueprint.WorkflowCreateRequest({
            name: "trigger-test",
            workflow_json: "{}",
            trigger_type: "manual",
            trigger_config: "",
            sandbox_config_json: "{}"
        });
        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_CREATE(), 510, operator1,
            abi.encode(req), encodeJsonOutputs("{}")
        );

        vm.warp(1000);

        AgentSandboxBlueprint.WorkflowControlRequest memory ctrl = AgentSandboxBlueprint.WorkflowControlRequest({
            workflow_id: 510
        });

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.WorkflowTriggered(510, 1000);

        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_TRIGGER(), 511, operator1,
            abi.encode(ctrl), encodeJsonOutputs("{}")
        );

        AgentSandboxBlueprint.WorkflowConfig memory config = blueprint.getWorkflow(510);
        assertEq(config.last_triggered_at, 1000);
    }

    function test_workflowCancelDeactivates() public {
        AgentSandboxBlueprint.WorkflowCreateRequest memory req = AgentSandboxBlueprint.WorkflowCreateRequest({
            name: "cancel-test",
            workflow_json: "{}",
            trigger_type: "cron",
            trigger_config: "0 * * * *",
            sandbox_config_json: "{}"
        });
        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_CREATE(), 520, operator1,
            abi.encode(req), encodeJsonOutputs("{}")
        );

        assertTrue(blueprint.getWorkflow(520).active);

        AgentSandboxBlueprint.WorkflowControlRequest memory ctrl = AgentSandboxBlueprint.WorkflowControlRequest({
            workflow_id: 520
        });

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.WorkflowCanceled(520, uint64(block.timestamp));

        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_CANCEL(), 521, operator1,
            abi.encode(ctrl), encodeJsonOutputs("{}")
        );

        assertFalse(blueprint.getWorkflow(520).active);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SANDBOX ALREADY EXISTS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_createResultRevertsDuplicateSandboxId() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 800, operator1, "sandbox-dup");

        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 801, encodeSandboxCreateInputs());

        bytes32 dupHash = keccak256(bytes("sandbox-dup"));
        vm.prank(tangleCore);
        vm.expectRevert(
            abi.encodeWithSelector(AgentSandboxBlueprint.SandboxAlreadyExists.selector, dupHash)
        );
        blueprint.onJobResult(
            1, 0, 801, operator1,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("sandbox-dup", "{}")
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // getRequiredResultCount
    // ═══════════════════════════════════════════════════════════════════════════

    function test_getRequiredResultCountAlwaysReturnsOne() public view {
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_SANDBOX_CREATE()), 1);
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_SANDBOX_DELETE()), 1);
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_WORKFLOW_CREATE()), 1);
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_PROVISION()), 1);
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_DEPROVISION()), 1);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MODE FLAGS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_cloudModeIsDefault() public view {
        assertFalse(blueprint.instanceMode());
        assertFalse(blueprint.teeRequired());
    }

    function test_setInstanceMode() public {
        vm.prank(blueprintOwner);
        blueprint.setInstanceMode(true);
        assertTrue(blueprint.instanceMode());
    }

    function test_setTeeRequired() public {
        vm.prank(blueprintOwner);
        blueprint.setTeeRequired(true);
        assertTrue(blueprint.teeRequired());
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    function test_jobMetadata() public view {
        uint8[] memory ids = blueprint.jobIds();
        assertEq(ids.length, 7);
        assertEq(ids[0], 0); // SANDBOX_CREATE
        assertEq(ids[6], 6); // DEPROVISION

        assertTrue(blueprint.supportsJob(0));
        assertTrue(blueprint.supportsJob(6));
        assertFalse(blueprint.supportsJob(7));

        assertEq(blueprint.jobCount(), 7);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════════════

    function _createSandbox(
        uint64 serviceId,
        uint64 callId,
        address operator,
        string memory sandboxId
    ) internal {
        address[3] memory allOps = [operator1, operator2, operator3];
        for (uint256 i = 0; i < 3; i++) {
            if (allOps[i] != operator && allOps[i] != address(0)) {
                mockDelegation.setActive(allOps[i], false);
            }
        }

        simulateJobCall(serviceId, blueprint.JOB_SANDBOX_CREATE(), callId, encodeSandboxCreateInputs());

        for (uint256 i = 0; i < 3; i++) {
            if (allOps[i] != operator && allOps[i] != address(0)) {
                mockDelegation.setActive(allOps[i], true);
            }
        }

        simulateJobResult(
            serviceId,
            blueprint.JOB_SANDBOX_CREATE(),
            callId,
            operator,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs(sandboxId, "{}")
        );
    }
}
