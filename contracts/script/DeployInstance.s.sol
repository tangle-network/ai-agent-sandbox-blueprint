// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentSandboxBlueprint.sol";

/// @notice Convenience script: deploys the unified contract in instance mode.
contract DeployInstanceBlueprint is Script {
    function run() external {
        vm.startBroadcast();

        AgentSandboxBlueprint blueprint = new AgentSandboxBlueprint(address(0), true, false);
        console.log("AgentSandboxBlueprint (instance) deployed at:", address(blueprint));

        vm.stopBroadcast();
    }
}
