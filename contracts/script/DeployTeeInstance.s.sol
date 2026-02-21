// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentSandboxBlueprint.sol";

/// @notice Convenience script: deploys the unified contract in TEE instance mode.
contract DeployTeeInstanceBlueprint is Script {
    function run() external {
        vm.startBroadcast();

        AgentSandboxBlueprint blueprint = new AgentSandboxBlueprint(address(0), true, true);
        console.log("AgentSandboxBlueprint (TEE instance) deployed at:", address(blueprint));

        vm.stopBroadcast();
    }
}
