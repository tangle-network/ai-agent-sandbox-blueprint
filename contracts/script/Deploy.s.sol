// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentSandboxBlueprint.sol";

contract DeployBlueprint is Script {
    function run() external {
        address restaking = vm.envOr("RESTAKING_ADDRESS", address(0));
        uint32 minOps = uint32(vm.envOr("MIN_OPERATORS", uint256(1)));
        uint32 maxOps = uint32(vm.envOr("MAX_OPERATORS", uint256(100)));
        uint32 defaultOps = uint32(vm.envOr("DEFAULT_OPERATOR_COUNT", uint256(1)));
        uint32 defaultCapacity = uint32(vm.envOr("DEFAULT_MAX_CAPACITY", uint256(100)));

        vm.startBroadcast();

        AgentSandboxBlueprint blueprint = new AgentSandboxBlueprint(restaking);
        console.log("AgentSandboxBlueprint deployed at:", address(blueprint));

        blueprint.setOperatorSelectionConfig(minOps, maxOps, defaultOps);
        console.log("Operator selection config: min=%d max=%d default=%d", minOps, maxOps, defaultOps);

        blueprint.setDefaultMaxCapacity(defaultCapacity);
        console.log("Default max capacity:", defaultCapacity);

        vm.stopBroadcast();
    }
}
