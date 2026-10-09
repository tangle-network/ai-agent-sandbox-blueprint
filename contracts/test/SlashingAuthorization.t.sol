// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Test.sol";
import {MockMultiAssetDelegation} from "./helpers/Setup.sol";
import {AgentSandboxBlueprint} from "../src/AgentSandboxBlueprint.sol";
import {BlueprintServiceManagerBase} from "tnt-core/BlueprintServiceManagerBase.sol";
import {SlashingTypes} from "tangle-slashing/SlashingTypes.sol";

contract SlashingAuthorizationTest is Test {
    address private constant tangleCore = address(0x7A);
    address private constant blueprintOwner = address(0xBB);
    address private constant operator1 = address(0x1001);

    function variant(uint256 mode) internal returns (AgentSandboxBlueprint target) {
        target = new AgentSandboxBlueprint(address(new MockMultiAssetDelegation()), mode > 0, mode == 2, address(0));
        target.onBlueprintCreated(42, blueprintOwner, tangleCore);
    }

    function seedEvidence(AgentSandboxBlueprint target) internal returns (bytes32) {
        return target.submitEvidence(7, operator1, SlashingTypes.ViolationType.SERVICE_NOT_DELIVERED, "evidence");
    }

    function test_slashingCallbacksRejectNonTangle() public {
        for (uint256 mode; mode < 3; ++mode) {
            AgentSandboxBlueprint target = variant(mode);
            bytes32 evidenceHash = seedEvidence(target);
            address[2] memory callers = [address(0xBAD), blueprintOwner];
            for (uint256 i; i < callers.length; ++i) {
                bytes memory expected = abi.encodeWithSelector(
                    BlueprintServiceManagerBase.OnlyTangleAllowed.selector, callers[i], tangleCore
                );
                vm.expectRevert(expected);
                vm.prank(callers[i]);
                target.onUnappliedSlash(7, abi.encodePacked(operator1), 5);
                vm.expectRevert(expected);
                vm.prank(callers[i]);
                target.onSlash(7, abi.encodePacked(operator1), 5);
            }
            assertEq(target.slashHistory(operator1, 7), 0);
            assertEq(target.totalSlashes(operator1), 0);
            assertEq(target.pendingEvidence(7, operator1), evidenceHash);
        }
    }

    function test_slashingCallbacksAcceptConfiguredTangle() public {
        for (uint256 mode; mode < 3; ++mode) {
            AgentSandboxBlueprint target = variant(mode);
            bytes32 evidenceHash = seedEvidence(target);
            vm.prank(tangleCore);
            target.onUnappliedSlash(7, abi.encodePacked(operator1), 5);
            assertEq(target.pendingEvidence(7, operator1), evidenceHash);
            assertEq(target.totalSlashes(operator1), 0);
            vm.prank(tangleCore);
            target.onSlash(7, abi.encodePacked(operator1), 5);
            assertEq(target.slashHistory(operator1, 7), 1);
            assertEq(target.totalSlashes(operator1), 1);
            assertEq(target.pendingEvidence(7, operator1), bytes32(0));
        }
    }
}
