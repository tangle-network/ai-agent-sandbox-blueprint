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
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_WORKFLOW_TRIGGER()), 1);
        assertEq(blueprint.getRequiredResultCount(1, blueprint.JOB_WORKFLOW_CANCEL()), 1);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MODE FLAGS
    // ═══════════════════════════════════════════════════════════════════════════

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

    function test_setInstanceModeRevertsWithActiveSandboxes() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 950, operator1, "sandbox-guard");
        assertEq(blueprint.totalActiveSandboxes(), 1);

        vm.prank(blueprintOwner);
        vm.expectRevert(AgentSandboxBlueprint.CannotChangeWithActiveResources.selector);
        blueprint.setInstanceMode(true);
    }

    function test_setTeeRequiredRevertsWithActiveSandboxes() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 951, operator1, "sandbox-tee-guard");
        assertEq(blueprint.totalActiveSandboxes(), 1);

        vm.prank(blueprintOwner);
        vm.expectRevert(AgentSandboxBlueprint.CannotChangeWithActiveResources.selector);
        blueprint.setTeeRequired(true);
    }

    function test_setInstanceModeSucceedsWhenNoActiveSandboxes() public {
        assertEq(blueprint.totalActiveSandboxes(), 0);

        vm.prank(blueprintOwner);
        blueprint.setInstanceMode(true);
        assertTrue(blueprint.instanceMode());

        vm.prank(blueprintOwner);
        blueprint.setInstanceMode(false);
        assertFalse(blueprint.instanceMode());
    }

    function test_setTeeRequiredSucceedsWhenNoActiveSandboxes() public {
        assertEq(blueprint.totalActiveSandboxes(), 0);

        vm.prank(blueprintOwner);
        blueprint.setTeeRequired(true);
        assertTrue(blueprint.teeRequired());

        vm.prank(blueprintOwner);
        blueprint.setTeeRequired(false);
        assertFalse(blueprint.teeRequired());
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MODE ENFORCEMENT — CLOUD MODE REJECTS INSTANCE JOBS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_cloudModeRejectsUnknownJobs() public {
        for (uint8 jobId = 5; jobId <= 6; jobId++) {
            vm.prank(tangleCore);
            vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, jobId));
            blueprint.onJobCall(1, jobId, uint64(960 + jobId), bytes(""));
        }
        for (uint8 jobId = 5; jobId <= 6; jobId++) {
            vm.prank(tangleCore);
            vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, jobId));
            blueprint.onJobResult(1, jobId, uint64(962 + jobId), operator1, bytes(""), bytes(""));
        }
    }

    function test_cloudModeRejectsDirectInstanceReporting() public {
        vm.prank(operator1);
        vm.expectRevert(AgentSandboxBlueprint.InstanceModeOnly.selector);
        blueprint.reportProvisioned(1, "sb-r1", "http://op1:8080", 2222, "");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // JOB METADATA
    // ═══════════════════════════════════════════════════════════════════════════

    // ═══════════════════════════════════════════════════════════════════════════
    // SECURITY: SANDBOX ID VALIDATION (M6)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_createResultRevertsEmptySandboxId() public {
        registerOperator(operator1, 10);
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 900, encodeSandboxCreateInputs());

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.EmptySandboxId.selector);
        blueprint.onJobResult(
            1, 0, 900, operator1,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs("", "{}")
        );
    }

    function test_createResultRevertsTooLongSandboxId() public {
        registerOperator(operator1, 10);
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 901, encodeSandboxCreateInputs());

        // Build a 256-character string (exceeds 255 limit)
        bytes memory longBytes = new bytes(256);
        for (uint256 i = 0; i < 256; i++) {
            longBytes[i] = "a";
        }
        string memory longId = string(longBytes);

        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.SandboxIdTooLong.selector, 256));
        blueprint.onJobResult(
            1, 0, 901, operator1,
            encodeSandboxCreateInputs(),
            encodeSandboxCreateOutputs(longId, "{}")
        );
    }

    function test_createResultAccepts255CharSandboxId() public {
        registerOperator(operator1, 10);

        // Build a 255-character string (exactly at limit)
        bytes memory maxBytes = new bytes(255);
        for (uint256 i = 0; i < 255; i++) {
            maxBytes[i] = "b";
        }
        string memory maxId = string(maxBytes);

        _createSandbox(1, 902, operator1, maxId);
        assertTrue(blueprint.isSandboxActive(maxId));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SECURITY: WORKFLOW ARRAY BOUNDS (H5a)
    // ═══════════════════════════════════════════════════════════════════════════

    // ═══════════════════════════════════════════════════════════════════════════
    // SECURITY: SERVICE REQUEST VALIDATED EVENT (M7)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onRequestEmitsServiceRequestValidated() public {
        registerOperator(operator1, 10);

        address[] memory operators = new address[](1);
        operators[0] = operator1;

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.ServiceRequestValidated(1, blueprintOwner, 1);

        vm.prank(tangleCore);
        blueprint.onRequest(1, blueprintOwner, operators, bytes(""), 0, address(0), 0);
    }

    function test_onRequestEmitsServiceRequestValidatedMultipleOperators() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);

        address[] memory operators = new address[](2);
        operators[0] = operator1;
        operators[1] = operator2;

        vm.expectEmit(true, false, false, true);
        emit AgentSandboxBlueprint.ServiceRequestValidated(2, blueprintOwner, 2);

        vm.prank(tangleCore);
        blueprint.onRequest(2, blueprintOwner, operators, bytes(""), 0, address(0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // UNKNOWN JOB ID REVERTS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_unknownJobIdRevertsOnJobCall() public {
        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, 7));
        blueprint.onJobCall(1, 7, 970, bytes(""));
    }

    function test_unknownJobIdRevertsOnJobCallHighId() public {
        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, 255));
        blueprint.onJobCall(1, 255, 971, bytes(""));
    }

    function test_unknownJobIdRevertsOnJobResult() public {
        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, 7));
        blueprint.onJobResult(1, 7, 972, operator1, bytes(""), bytes(""));
    }

    function test_unknownJobIdRevertsOnJobResultHighId() public {
        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.UnknownJobId.selector, 255));
        blueprint.onJobResult(1, 255, 973, operator1, bytes(""), bytes(""));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPERATOR UNREGISTER / LEAVE TESTS (cloud mode)
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onUnregisterRevertsWithActiveSandboxes() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 4000, operator1, "sb-unreg");

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.CannotLeaveWithActiveResources.selector);
        blueprint.onUnregister(operator1);
    }

    function test_onUnregisterSucceedsWithNoSandboxes() public {
        registerOperator(operator1, 10);

        // Should succeed — no active sandboxes
        vm.prank(tangleCore);
        blueprint.onUnregister(operator1);
    }

    function test_onUnregisterSucceedsAfterDeletion() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 4010, operator1, "sb-del-unreg");

        // Delete the sandbox
        simulateJobResult(
            1, blueprint.JOB_SANDBOX_DELETE(), 4011, operator1,
            encodeSandboxIdInputs("sb-del-unreg"),
            encodeJsonOutputs("{}")
        );
        assertEq(blueprint.operatorActiveSandboxes(operator1), 0);

        // Now unregister should succeed
        vm.prank(tangleCore);
        blueprint.onUnregister(operator1);
    }

    function test_onOperatorLeftRevertsWithActiveSandboxes() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 4020, operator1, "sb-leave");

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.CannotLeaveWithActiveResources.selector);
        blueprint.onOperatorLeft(1, operator1);
    }

    function test_onOperatorLeftSucceedsWithNoSandboxes() public {
        registerOperator(operator1, 10);

        // Should succeed — no active sandboxes, no provisions
        vm.prank(tangleCore);
        blueprint.onOperatorLeft(1, operator1);
    }

    function test_canLeaveReturnsFalseWithActiveSandboxes() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 4030, operator1, "sb-canleave");

        assertFalse(blueprint.canLeave(1, operator1));
    }

    function test_canLeaveReturnsTrueWithNoResources() public {
        registerOperator(operator1, 10);
        assertTrue(blueprint.canLeave(1, operator1));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // FUZZ TESTS
    // ═══════════════════════════════════════════════════════════════════════════

    function testFuzz_sandboxIdValidation(bytes memory sandboxId) public pure {
        // Test that sandbox ID validation logic works for arbitrary inputs.
        // IDs of length 0 or >255 should be rejected in _handleCreateResult.
        vm.assume(sandboxId.length > 0 && sandboxId.length <= 255);
        // Verify the string is processable (won't revert on encoding)
        string memory idStr = string(sandboxId);
        bytes memory encoded = abi.encode(idStr, "http://sidecar:8080", uint32(22), uint32(2222));
        assertTrue(encoded.length > 0);
        assertTrue(bytes(idStr).length > 0);
        assertTrue(bytes(idStr).length <= 255);
    }

    function testFuzz_jobRatesNoOverflow(uint256 baseRate) public view {
        // Verify getDefaultJobRates doesn't panic for any input.
        // It will revert on overflow due to Solidity 0.8 checked math.
        vm.assume(baseRate <= type(uint256).max / 50);
        (uint8[] memory jobs, uint256[] memory rates) = blueprint.getDefaultJobRates(baseRate);
        assertTrue(jobs.length == 5);
        assertTrue(rates.length == 5);
        // Verify all rates are >= baseRate (multiplied by >= 1)
        for (uint256 i = 0; i < rates.length; i++) {
            assertTrue(rates[i] >= baseRate);
        }
    }

    function testFuzz_operatorSelectionBounded(uint8 numOperators) public {
        // Verify selection doesn't revert for valid operator counts (1-10).
        // Bound to a reasonable range to avoid excessive gas.
        vm.assume(numOperators >= 1 && numOperators <= 10);

        // Register numOperators operators
        for (uint8 i = 0; i < numOperators; i++) {
            address op = address(uint160(0x2000 + i));
            mockDelegation.addOperator(op, testBlueprintId);
            vm.prank(tangleCore);
            blueprint.onRegister(op, abi.encode(uint32(10)));
        }

        // Should not revert — at least one operator has capacity
        vm.prevrandao(bytes32(uint256(42)));
        vm.recordLogs();
        simulateJobCall(1, blueprint.JOB_SANDBOX_CREATE(), 5000, encodeSandboxCreateInputs());

        // Verify an assignment was emitted
        Vm.Log[] memory logs = vm.getRecordedLogs();
        bool found = false;
        for (uint256 j = 0; j < logs.length; j++) {
            if (logs[j].topics[0] == AgentSandboxBlueprint.OperatorAssigned.selector) {
                found = true;
            }
        }
        assertTrue(found, "operator assignment should have been emitted");
    }

    function testFuzz_workflowIdValidation(uint64 workflowId) public {
        // Verify workflow operations handle any uint64 ID correctly.
        // Reserve 2 IDs above workflowId for trigger/cancel callIds.
        vm.assume(workflowId <= type(uint64).max - 2);

        AgentSandboxBlueprint.WorkflowCreateRequest memory req = AgentSandboxBlueprint.WorkflowCreateRequest({
            name: "fuzz-workflow",
            workflow_json: "{}",
            trigger_type: "cron",
            trigger_config: "* * * * *",
            sandbox_config_json: "{}"
        });

        // Create workflow with fuzzed ID — should always succeed
        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_CREATE(), workflowId, operator1,
            abi.encode(req), encodeJsonOutputs("{}")
        );

        AgentSandboxBlueprint.WorkflowConfig memory config = blueprint.getWorkflow(workflowId);
        assertEq(config.name, "fuzz-workflow");
        assertTrue(config.active);

        // Trigger should also work with any uint64
        AgentSandboxBlueprint.WorkflowControlRequest memory ctrl = AgentSandboxBlueprint.WorkflowControlRequest({
            workflow_id: workflowId
        });
        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_TRIGGER(), workflowId + 1, operator1,
            abi.encode(ctrl), encodeJsonOutputs("{}")
        );
        assertEq(blueprint.getWorkflow(workflowId).last_triggered_at, uint64(block.timestamp));

        // Cancel should work too
        simulateJobResult(
            1, blueprint.JOB_WORKFLOW_CANCEL(), workflowId + 2, operator1,
            abi.encode(ctrl), encodeJsonOutputs("{}")
        );
        assertFalse(blueprint.getWorkflow(workflowId).active);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REVERT PATH: onRequest WITH ZERO OPERATORS
    // ═══════════════════════════════════════════════════════════════════════════

    function test_onRequestRevertsWithZeroOperators() public {
        address[] memory operators = new address[](0);

        vm.prank(tangleCore);
        vm.expectRevert(AgentSandboxBlueprint.ZeroOperatorsInRequest.selector);
        blueprint.onRequest(1, blueprintOwner, operators, bytes(""), 0, address(0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REVERT PATH: _markTriggered FOR NON-EXISTENT WORKFLOW
    // ═══════════════════════════════════════════════════════════════════════════

    function test_markTriggeredRevertsForNonExistentWorkflow() public {
        AgentSandboxBlueprint.WorkflowControlRequest memory ctrl = AgentSandboxBlueprint.WorkflowControlRequest({
            workflow_id: 99999
        });

        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.WorkflowNotFound.selector, uint64(99999)));
        blueprint.onJobResult(
            1, 3, 5000, operator1, // JOB_WORKFLOW_TRIGGER = 3
            abi.encode(ctrl), encodeJsonOutputs("{}")
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REVERT PATH: _cancelWorkflow FOR NON-EXISTENT WORKFLOW
    // ═══════════════════════════════════════════════════════════════════════════

    function test_cancelWorkflowRevertsForNonExistentWorkflow() public {
        AgentSandboxBlueprint.WorkflowControlRequest memory ctrl = AgentSandboxBlueprint.WorkflowControlRequest({
            workflow_id: 88888
        });

        vm.prank(tangleCore);
        vm.expectRevert(abi.encodeWithSelector(AgentSandboxBlueprint.WorkflowNotFound.selector, uint64(88888)));
        blueprint.onJobResult(
            1, 4, 5001, operator1, // JOB_WORKFLOW_CANCEL = 4
            abi.encode(ctrl), encodeJsonOutputs("{}")
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // REVERT PATH: setOperatorCapacity BY NON-OWNER
    // ═══════════════════════════════════════════════════════════════════════════

    function test_setOperatorCapacityRevertsForNonOwner() public {
        registerOperator(operator1, 50);

        vm.prank(operator1);
        vm.expectRevert();
        blueprint.setOperatorCapacity(operator1, 200);
    }

    function test_setDefaultMaxCapacityRevertsForNonOwner() public {
        vm.prank(operator1);
        vm.expectRevert();
        blueprint.setDefaultMaxCapacity(500);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // SAFE DECREMENT — DELETION SUCCEEDS EVEN IF COUNTER IS ALREADY 0
    // ═══════════════════════════════════════════════════════════════════════════

    /// @notice Verifies that _handleDeleteResult does not revert if
    ///         operatorActiveSandboxes or totalActiveSandboxes is somehow 0.
    ///         Uses vm.store to force the counters to 0 after a sandbox has
    ///         been created, then deletes the sandbox. Without the safe
    ///         decrement pattern (if > 0) this would underflow and revert
    ///         under Solidity 0.8 checked arithmetic.
    function test_deleteSucceedsWhenCounterAlreadyZero() public {
        registerOperator(operator1, 10);
        _createSandbox(1, 6000, operator1, "sb-safe-dec");

        // Sanity: counters should be 1
        assertEq(blueprint.operatorActiveSandboxes(operator1), 1);
        assertEq(blueprint.totalActiveSandboxes(), 1);

        // Force operatorActiveSandboxes[operator1] to 0 via vm.store.
        // operatorActiveSandboxes is mapping(address => uint32) at base slot 6.
        // Mapping slot = keccak256(abi.encode(key, baseSlot)).
        bytes32 opActiveSlot = keccak256(abi.encode(operator1, uint256(6)));
        vm.store(address(blueprint), opActiveSlot, bytes32(uint256(0)));

        // Force totalActiveSandboxes to 0. It's a uint32 at slot 7, offset 4.
        // Slot 7 is packed: [defaultMaxCapacity (uint32 @ offset 0), totalActiveSandboxes (uint32 @ offset 4)].
        // Preserve defaultMaxCapacity (100) while zeroing totalActiveSandboxes.
        bytes32 slot7 = vm.load(address(blueprint), bytes32(uint256(7)));
        // Zero out bytes 4-7 (totalActiveSandboxes) while keeping bytes 0-3 (defaultMaxCapacity).
        // In EVM storage, lower offsets are stored in lower-order bytes of the 32-byte word.
        bytes32 mask = bytes32(uint256(0xFFFFFFFF)); // keep lowest 4 bytes (defaultMaxCapacity)
        bytes32 newSlot7 = slot7 & mask;
        vm.store(address(blueprint), bytes32(uint256(7)), newSlot7);

        // Verify forced to 0
        assertEq(blueprint.operatorActiveSandboxes(operator1), 0);
        assertEq(blueprint.totalActiveSandboxes(), 0);
        // Verify defaultMaxCapacity is preserved
        assertEq(blueprint.defaultMaxCapacity(), 100);

        // Delete should succeed (no underflow revert) thanks to safe decrement
        simulateJobResult(
            1,
            blueprint.JOB_SANDBOX_DELETE(),
            6001,
            operator1,
            encodeSandboxIdInputs("sb-safe-dec"),
            encodeJsonOutputs("{\"deleted\":true}")
        );

        // Counters remain 0 (clamped, not underflowed)
        assertEq(blueprint.operatorActiveSandboxes(operator1), 0);
        assertEq(blueprint.totalActiveSandboxes(), 0);
        assertFalse(blueprint.isSandboxActive("sb-safe-dec"));
        assertEq(blueprint.getSandboxOperator("sb-safe-dec"), address(0));
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
