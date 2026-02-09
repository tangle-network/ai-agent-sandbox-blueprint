// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "./helpers/Setup.sol";

contract OperatorSelectionTest is BlueprintTestSetup {

    function setUp() public override {
        super.setUp();
        // Configure operator selection bounds
        vm.prank(blueprintOwner);
        blueprint.setOperatorSelectionConfig(1, 10, 1);
    }

    function test_deterministicSelection() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);
        registerOperator(operator3, 10);

        bytes32 seed = keccak256("test-seed");
        address[] memory result1 = blueprint.previewOperatorSelection(2, seed);
        address[] memory result2 = blueprint.previewOperatorSelection(2, seed);

        assertEq(result1.length, 2);
        assertEq(result2.length, 2);
        assertEq(result1[0], result2[0], "same seed should produce same first selection");
        assertEq(result1[1], result2[1], "same seed should produce same second selection");
    }

    function test_selectionDifferentSeeds() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);
        registerOperator(operator3, 10);

        address[] memory result1 = blueprint.previewOperatorSelection(1, keccak256("seed-a"));
        address[] memory result2 = blueprint.previewOperatorSelection(1, keccak256("seed-b"));

        assertEq(result1.length, 1);
        assertEq(result2.length, 1);
        // With 3 operators and different seeds, they could be the same by chance,
        // but over many seeds they should differ. Just verify both return valid operators.
        assertTrue(
            result1[0] == operator1 || result1[0] == operator2 || result1[0] == operator3,
            "result1 should be a valid operator"
        );
        assertTrue(
            result2[0] == operator1 || result2[0] == operator2 || result2[0] == operator3,
            "result2 should be a valid operator"
        );
    }

    function test_selectionRejectsIneligible() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);

        // Deactivate operator1
        mockDelegation.setActive(operator1, false);

        address[] memory result = blueprint.previewOperatorSelection(1, keccak256("test"));
        assertEq(result.length, 1);
        assertEq(result[0], operator2, "inactive operator should not be selected");
    }

    function test_notEnoughOperators() public {
        registerOperator(operator1, 10);

        vm.expectRevert(
            abi.encodeWithSelector(
                OperatorSelectionBase.NotEnoughEligibleOperators.selector,
                uint32(3),
                uint32(1)
            )
        );
        blueprint.previewOperatorSelection(3, keccak256("test"));
    }

    function test_eligibleOperatorsPublic() public {
        registerOperator(operator1, 10);
        registerOperator(operator2, 10);
        mockDelegation.setActive(operator1, false);

        address[] memory eligible = blueprint.eligibleOperators();
        assertEq(eligible.length, 1);
        assertEq(eligible[0], operator2);
    }
}
