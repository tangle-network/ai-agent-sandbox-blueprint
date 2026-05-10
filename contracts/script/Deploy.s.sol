// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentSandboxBlueprint.sol";

contract DeployBlueprint is Script {
    function run() external {
        address restaking = vm.envOr("RESTAKING_ADDRESS", address(0));
        bool isInstance = vm.envOr("INSTANCE_MODE", false);
        bool isTee = vm.envOr("TEE_REQUIRED", false);
        uint32 minOps = uint32(vm.envOr("MIN_OPERATORS", uint256(1)));
        uint32 maxOps = uint32(vm.envOr("MAX_OPERATORS", uint256(100)));
        uint32 defaultOps = uint32(vm.envOr("DEFAULT_OPERATOR_COUNT", uint256(1)));
        uint32 defaultCapacity = uint32(vm.envOr("DEFAULT_MAX_CAPACITY", uint256(100)));

        // Cloud-mode requires a real restaking address — without it, capacity-
        // weighted operator selection (`_selectByCapacity`) reverts and
        // `JOB_SANDBOX_CREATE` is non-functional. Fail the deploy at script
        // time so a missing env var doesn't silently ship a broken cloud
        // blueprint. Instance / TEE-instance scripts deliberately pass
        // address(0) — they don't go through this gate.
        require(
            isInstance || restaking != address(0),
            "Deploy: RESTAKING_ADDRESS required for cloud-mode (set INSTANCE_MODE=true to skip)"
        );

        vm.startBroadcast();

        AgentSandboxBlueprint blueprint = new AgentSandboxBlueprint(restaking, isInstance, isTee);
        console.log("AgentSandboxBlueprint deployed at:", address(blueprint));
        console.log("  instanceMode:", isInstance);
        console.log("  teeRequired:", isTee);

        if (!isInstance) {
            blueprint.setOperatorSelectionConfig(minOps, maxOps, defaultOps);
            console.log("Operator selection config: min=%d max=%d default=%d", minOps, maxOps, defaultOps);

            blueprint.setDefaultMaxCapacity(defaultCapacity);
            console.log("Default max capacity:", defaultCapacity);
        }

        vm.stopBroadcast();
    }
}
