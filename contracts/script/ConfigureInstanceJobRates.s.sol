// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "../src/AgentInstanceBlueprint.sol";

/**
 * @title ConfigureInstanceJobRates
 * @notice Post-registration script to set per-job pricing on the Tangle contract
 *         for the AI Agent Instance Blueprint.
 *
 *         After deploying the BSM and registering the blueprint on Tangle, run:
 *
 *         BASE_RATE=1000000000000000 \     # 0.001 TNT (adjust to token price)
 *         BLUEPRINT_ID=<id> \
 *         TANGLE_ADDRESS=<proxy> \
 *         BSM_ADDRESS=<bsm> \
 *         forge script contracts/script/ConfigureInstanceJobRates.s.sol:ConfigureInstanceJobRates \
 *           --rpc-url $RPC_URL --broadcast
 *
 *         Base rate guide (assuming 1 TNT ≈ $1):
 *           1e15  = 0.001 TNT ≈ $0.001 per EXEC  → PROVISION ≈ $0.05, TASK ≈ $0.25
 *           1e14  = 0.0001 TNT ≈ $0.0001 per EXEC → PROVISION ≈ $0.005, TASK ≈ $0.025
 *           1e16  = 0.01 TNT ≈ $0.01 per EXEC   → PROVISION ≈ $0.50, TASK ≈ $2.50
 */

interface ITangleSetJobRates {
    function setJobEventRates(uint64 blueprintId, uint8[] calldata jobIndexes, uint256[] calldata rates) external;
    function getJobEventRate(uint64 blueprintId, uint8 jobIndex) external view returns (uint256 rate);
}

contract ConfigureInstanceJobRates is Script {
    function run() external {
        uint256 baseRate = vm.envUint("BASE_RATE");
        uint64 blueprintId = uint64(vm.envUint("BLUEPRINT_ID"));
        address tangleAddress = vm.envAddress("TANGLE_ADDRESS");
        address bsmAddress = vm.envAddress("BSM_ADDRESS");

        AgentInstanceBlueprint bsm = AgentInstanceBlueprint(payable(bsmAddress));
        ITangleSetJobRates tangle = ITangleSetJobRates(tangleAddress);

        (uint8[] memory jobIndexes, uint256[] memory rates) = bsm.getDefaultJobRates(baseRate);

        console.log("=== AI Agent Instance Blueprint: Per-Job Pricing ===");
        console.log("Blueprint ID:", blueprintId);
        console.log("Base rate (wei):", baseRate);
        console.log("");

        string[8] memory jobNames = [
            "PROVISION", "EXEC", "PROMPT", "TASK",
            "SSH_PROVISION", "SSH_REVOKE", "SNAPSHOT", "DEPROVISION"
        ];

        for (uint256 i = 0; i < 8; i++) {
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
        console.log("Instance job rates configured successfully.");
    }
}
