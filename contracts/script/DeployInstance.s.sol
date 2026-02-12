// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentInstanceBlueprint.sol";

contract DeployInstanceBlueprint is Script {
    function run() external {
        vm.startBroadcast();

        AgentInstanceBlueprint blueprint = new AgentInstanceBlueprint();
        console.log("AgentInstanceBlueprint deployed at:", address(blueprint));

        vm.stopBroadcast();
    }
}
