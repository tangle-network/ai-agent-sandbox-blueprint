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
