// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentSandboxBlueprint.sol";

/**
 * @title ConfigureJobRates
 * @notice Post-registration script to set per-job pricing on the Tangle contract.
 *         Works for all 3 deployment modes (cloud, instance, TEE instance).
 *
 *         After deploying the BSM and registering the blueprint on Tangle, run:
 *
 *         BASE_RATE=1000000000000000 \     # 0.001 TNT (adjust to token price)
 *         BLUEPRINT_ID=<id> \
 *         TANGLE_ADDRESS=<proxy> \
 *         BSM_ADDRESS=<bsm> \
 *         forge script contracts/script/ConfigureJobRates.s.sol:ConfigureJobRates \
 *           --rpc-url $RPC_URL --broadcast
 */

interface ITangleSetJobRates {
    function setJobEventRates(uint64 blueprintId, uint8[] calldata jobIndexes, uint256[] calldata rates) external;
    function getJobEventRate(uint64 blueprintId, uint8 jobIndex) external view returns (uint256 rate);
}

contract ConfigureJobRates is Script {
    function run() external {
        uint256 baseRate = vm.envUint("BASE_RATE");
        uint64 blueprintId = uint64(vm.envUint("BLUEPRINT_ID"));
        address tangleAddress = vm.envAddress("TANGLE_ADDRESS");
        address bsmAddress = vm.envAddress("BSM_ADDRESS");

        AgentSandboxBlueprint bsm = AgentSandboxBlueprint(payable(bsmAddress));
        ITangleSetJobRates tangle = ITangleSetJobRates(tangleAddress);

        (uint8[] memory jobIndexes, uint256[] memory rates) = bsm.getDefaultJobRates(baseRate);

        console.log("=== AI Agent Sandbox Blueprint: Per-Job Pricing ===");
        console.log("Blueprint ID:", blueprintId);
        console.log("Base rate (wei):", baseRate);
        console.log("");

        string[7] memory jobNames = [
            "SANDBOX_CREATE", "SANDBOX_DELETE",
            "WORKFLOW_CREATE", "WORKFLOW_TRIGGER", "WORKFLOW_CANCEL",
            "PROVISION", "DEPROVISION"
        ];

        for (uint256 i = 0; i < 7; i++) {
            console.log(
                string.concat(
                    "  Job ", jobNames[i],
                    " (", vm.toString(jobIndexes[i]), "): ",
                    vm.toString(rates[i]), " wei"
                )
            );
        }

        vm.startBroadcast();
        tangle.setJobEventRates(blueprintId, jobIndexes, rates);
        vm.stopBroadcast();

        console.log("");
        console.log("Job rates configured successfully.");
    }
}
