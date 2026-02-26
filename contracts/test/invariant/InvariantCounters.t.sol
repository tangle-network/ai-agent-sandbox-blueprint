// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Test.sol";
import "../../src/AgentSandboxBlueprint.sol";
import "../helpers/Setup.sol";

/// @title SandboxHandler
/// @dev Handler contract that the invariant fuzzer calls. Wraps blueprint
///      operations so that the fuzzer can exercise random create/delete
///      sequences while the handler tracks its own shadow state for
///      comparison against the on-chain counters.
contract SandboxHandler is Test {
    AgentSandboxBlueprint public blueprint;
    MockMultiAssetDelegation public mockDelegation;
    address public tangleCore;

    // Pool of operators the handler cycles through
    address[] public operators;

    // Shadow state: active sandbox tracking
    string[] public activeSandboxIds;
    mapping(bytes32 => bool) public shadowActive;
    mapping(bytes32 => address) public shadowOperator;

    // Per-operator active count
    mapping(address => uint256) public perOperatorCount;

    // Monotonic counter for unique sandbox IDs and call IDs
    uint256 public nextId;
    uint64 public nextCallId;

    // Ghost counters exposed for invariant assertions
    uint256 public ghostTotalActive;

    constructor(
        AgentSandboxBlueprint _blueprint,
        MockMultiAssetDelegation _mockDelegation,
        address _tangleCore,
        address[] memory _operators
    ) {
        blueprint = _blueprint;
        mockDelegation = _mockDelegation;
        tangleCore = _tangleCore;
        operators = _operators;
        nextCallId = 100_000; // start high to avoid collisions with other tests
    }

    /// @notice Create a sandbox assigned to a deterministic operator.
    ///         The seed selects which operator gets the sandbox.
    function createSandbox(uint256 seed) external {
        if (operators.length == 0) return;

        // Pick an operator from the pool
        address operator = operators[seed % operators.length];

        // Check if this operator has capacity left on-chain
        uint32 maxCap = blueprint.operatorMaxCapacity(operator);
        uint32 activeCap = blueprint.operatorActiveSandboxes(operator);
        if (activeCap >= maxCap) return; // skip if full

        // Generate unique sandbox ID
        string memory sandboxId = string(abi.encodePacked("inv-sb-", vm.toString(nextId)));
        nextId++;

        // Ensure the sandbox ID doesn't already exist
        bytes32 sandboxHash = keccak256(bytes(sandboxId));
        if (shadowActive[sandboxHash]) return;

        uint64 callId = nextCallId++;

        // Deactivate all other operators so _selectByCapacity picks our target
        for (uint256 i = 0; i < operators.length; i++) {
            if (operators[i] != operator) {
                mockDelegation.setActive(operators[i], false);
            }
        }

        // Step 1: onJobCall — operator assignment
        vm.prank(tangleCore);
        blueprint.onJobCall(1, 0, callId, bytes("")); // JOB_SANDBOX_CREATE = 0

        // Re-activate all operators
        for (uint256 i = 0; i < operators.length; i++) {
            if (operators[i] != operator) {
                mockDelegation.setActive(operators[i], true);
            }
        }

        // Step 2: onJobResult — register sandbox
        vm.prank(tangleCore);
        blueprint.onJobResult(
            1, // serviceId
            0, // JOB_SANDBOX_CREATE
            callId,
            operator,
            bytes(""), // inputs (empty for create)
            abi.encode(sandboxId, "{}") // outputs: (string sandboxId, string json)
        );

        // Update shadow state
        activeSandboxIds.push(sandboxId);
        shadowActive[sandboxHash] = true;
        shadowOperator[sandboxHash] = operator;
        perOperatorCount[operator]++;
        ghostTotalActive++;
    }

    /// @notice Delete a sandbox from the active list.
    ///         The index selects which sandbox to delete (modulo list length).
    function deleteSandbox(uint256 index) external {
        if (activeSandboxIds.length == 0) return;

        uint256 idx = index % activeSandboxIds.length;
        string memory sandboxId = activeSandboxIds[idx];
        bytes32 sandboxHash = keccak256(bytes(sandboxId));
        address operator = shadowOperator[sandboxHash];

        uint64 callId = nextCallId++;

        // onJobResult for delete — inputs carry the sandboxId, outputs carry JSON
        vm.prank(tangleCore);
        blueprint.onJobResult(
            1, // serviceId
            1, // JOB_SANDBOX_DELETE
            callId,
            operator,
            abi.encode(sandboxId), // inputs: (string sandboxId)
            abi.encode("{}") // outputs: (string json)
        );

        // Update shadow state
        shadowActive[sandboxHash] = false;
        perOperatorCount[operator]--;
        ghostTotalActive--;
        delete shadowOperator[sandboxHash];

        // Swap-and-pop from activeSandboxIds
        uint256 lastIdx = activeSandboxIds.length - 1;
        if (idx != lastIdx) {
            activeSandboxIds[idx] = activeSandboxIds[lastIdx];
        }
        activeSandboxIds.pop();
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // VIEW HELPERS for invariant assertions
    // ═══════════════════════════════════════════════════════════════════════════

    function activeSandboxCount() external view returns (uint256) {
        return ghostTotalActive;
    }

    function activeSandboxIdsLength() external view returns (uint256) {
        return activeSandboxIds.length;
    }

    function getActiveSandboxId(uint256 index) external view returns (string memory) {
        return activeSandboxIds[index];
    }

    function getOperator(uint256 index) external view returns (address) {
        return operators[index];
    }

    function operatorCount() external view returns (uint256) {
        return operators.length;
    }

    function getPerOperatorCount(address op) external view returns (uint256) {
        return perOperatorCount[op];
    }
}

/// @title InvariantCountersTest
/// @dev Invariant test suite that verifies counter consistency under
///      randomized create/delete operation sequences in cloud mode.
contract InvariantCountersTest is Test {
    AgentSandboxBlueprint public blueprint;
    MockMultiAssetDelegation public mockDelegation;
    SandboxHandler public handler;

    address public tangleCore = address(0x7A);
    address public blueprintOwner = address(0xBB);
    uint64 public testBlueprintId = 42;

    // Three operators with generous capacity
    address public operator1 = address(0x1001);
    address public operator2 = address(0x1002);
    address public operator3 = address(0x1003);

    function setUp() public {
        // Deploy mock delegation and blueprint in cloud mode
        mockDelegation = new MockMultiAssetDelegation();
        blueprint = new AgentSandboxBlueprint(address(mockDelegation), false, false);
        blueprint.onBlueprintCreated(testBlueprintId, blueprintOwner, tangleCore);

        // Register all three operators with capacity 200 each
        address[] memory ops = new address[](3);
        ops[0] = operator1;
        ops[1] = operator2;
        ops[2] = operator3;

        for (uint256 i = 0; i < ops.length; i++) {
            mockDelegation.addOperator(ops[i], testBlueprintId);
            vm.prank(tangleCore);
            blueprint.onRegister(ops[i], abi.encode(uint32(200)));
        }

        // Deploy handler and target it for fuzzing
        handler = new SandboxHandler(blueprint, mockDelegation, tangleCore, ops);

        // Focus the fuzzer solely on the handler
        targetContract(address(handler));

        // Only call createSandbox and deleteSandbox
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = SandboxHandler.createSandbox.selector;
        selectors[1] = SandboxHandler.deleteSandbox.selector;
        targetSelector(FuzzSelector({addr: address(handler), selectors: selectors}));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INVARIANT: totalActiveSandboxes matches handler shadow count
    // ═══════════════════════════════════════════════════════════════════════════

    function invariant_totalActiveSandboxes() public view {
        assertEq(
            blueprint.totalActiveSandboxes(),
            handler.activeSandboxCount(),
            "totalActiveSandboxes != handler ghost count"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INVARIANT: per-operator sandbox counts match handler shadow counts
    // ═══════════════════════════════════════════════════════════════════════════

    function invariant_operatorSandboxesConsistent() public view {
        for (uint256 i = 0; i < handler.operatorCount(); i++) {
            address op = handler.getOperator(i);
            assertEq(
                blueprint.operatorActiveSandboxes(op),
                handler.getPerOperatorCount(op),
                "operatorActiveSandboxes mismatch"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INVARIANT: sandboxActive[hash] is true iff sandbox is in active list
    // ═══════════════════════════════════════════════════════════════════════════

    function invariant_sandboxActiveConsistent() public view {
        // Every sandbox in the active list should be marked active on-chain
        for (uint256 i = 0; i < handler.activeSandboxIdsLength(); i++) {
            string memory sid = handler.getActiveSandboxId(i);
            bytes32 h = keccak256(bytes(sid));
            assertTrue(
                blueprint.sandboxActive(h),
                "active sandbox not marked active on-chain"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INVARIANT: active list length equals ghost total
    // ═══════════════════════════════════════════════════════════════════════════

    function invariant_activeListLengthEqualsGhost() public view {
        assertEq(
            handler.activeSandboxIdsLength(),
            handler.activeSandboxCount(),
            "active list length != ghost counter"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INVARIANT: sum of per-operator counts == totalActiveSandboxes
    // ═══════════════════════════════════════════════════════════════════════════

    function invariant_sumOfOperatorCountsEqualTotal() public view {
        uint256 sum = 0;
        for (uint256 i = 0; i < handler.operatorCount(); i++) {
            address op = handler.getOperator(i);
            sum += handler.getPerOperatorCount(op);
        }
        assertEq(
            blueprint.totalActiveSandboxes(),
            sum,
            "sum of per-operator counts != totalActiveSandboxes"
        );
    }
}
