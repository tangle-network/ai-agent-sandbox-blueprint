// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentTeeInstanceBlueprint.sol";

contract DeployTeeInstanceBlueprint is Script {
    function run() external {
        vm.startBroadcast();

        AgentTeeInstanceBlueprint blueprint = new AgentTeeInstanceBlueprint();
        console.log("AgentTeeInstanceBlueprint deployed at:", address(blueprint));

        vm.stopBroadcast();
    }
}
