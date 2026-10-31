// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";
import "../src/AgentSandboxBlueprint.sol";

/// @notice Writes per-job rates for indexes the blueprint actually registered.
///         Rates are settlement-token units. The 50× cap is duplicated here so a
///         manager deployed before `maxBaseForDecimals` can still be repaired.
///         BASE_RATE overrides the 0.001-token default. SET_ASSET pins PAYMENT_TOKEN.
///         Writing 0 clears an override and restores eventRate. DRY_RUN skips broadcast.

interface IERC20Decimals {
    function decimals() external view returns (uint8);
}

interface ITangleSetJobRates {
    function setJobEventRates(uint64 blueprintId, uint8[] calldata jobIndexes, uint256[] calldata rates) external;
    function getJobEventRate(uint64 blueprintId, uint8 jobIndex) external view returns (uint256 rate);
    function setBlueprintSettlementAsset(uint64 blueprintId, address asset) external;
    function getBlueprintSettlementAsset(uint64 blueprintId) external view returns (address);
    function getBlueprintDefinition(uint64 blueprintId)
        external
        view
        returns (Types.BlueprintDefinition memory definition);
}

contract ConfigureJobRates is Script {
    function _defaultBase(uint8 decimals) internal pure returns (uint256) {
        if (decimals > 18) return 0;
        uint256 base = (10 ** uint256(decimals)) / 1000;
        return base == 0 ? 1 : base;
    }

    /// @dev Create is 50×. Cap the 1× base so that product is at most 1_000_000 whole tokens.
    function _maxBase(uint8 decimals) internal pure returns (uint256) {
        if (decimals > 18) return 0;
        return (1_000_000 * (10 ** uint256(decimals))) / 50;
    }

    function _jobName(uint8 jobId) internal pure returns (string memory) {
        if (jobId == 0) return "SANDBOX_CREATE";
        if (jobId == 1) return "SANDBOX_DELETE";
        if (jobId == 2) return "WORKFLOW_CREATE";
        if (jobId == 3) return "WORKFLOW_TRIGGER";
        if (jobId == 4) return "WORKFLOW_CANCEL";
        return "UNKNOWN";
    }

    function run() external {
        uint64 blueprintId = uint64(vm.envUint("BLUEPRINT_ID"));
        address tangleAddress = vm.envAddress("TANGLE_ADDRESS");
        address bsmAddress = vm.envAddress("BSM_ADDRESS");
        address paymentToken = vm.envOr("PAYMENT_TOKEN", address(0));
        bool setAsset = vm.envOr("SET_ASSET", false);

        AgentSandboxBlueprint bsm = AgentSandboxBlueprint(payable(bsmAddress));
        ITangleSetJobRates tangle = ITangleSetJobRates(tangleAddress);

        uint8 decimals;
        if (paymentToken != address(0)) {
            decimals = IERC20Decimals(paymentToken).decimals();
        } else {
            require(vm.envExists("DECIMALS"), "set PAYMENT_TOKEN or DECIMALS");
            decimals = uint8(vm.envUint("DECIMALS"));
        }
        // The cap lives in the script so a manager deployed before
        // `maxBaseForDecimals` can still be repaired. 50 is PRICE_MULT_SANDBOX_CREATE.
        uint256 baseRate = vm.envOr("BASE_RATE", _defaultBase(decimals));
        uint256 maxBase = _maxBase(decimals);
        require(baseRate > 0 && baseRate <= maxBase, "BASE_RATE out of range for settlement decimals");

        (uint8[] memory jobIndexes, uint256[] memory rates) = bsm.getDefaultJobRates(baseRate);
        uint256 jobCount = vm.envOr("JOB_COUNT", tangle.getBlueprintDefinition(blueprintId).jobs.length);
        (uint8[] memory writeIndexes, uint256[] memory writeRates) = _registeredRates(jobIndexes, rates, jobCount);
        require(writeIndexes.length > 0, "no registered jobs to price");

        console.log("=== AI Agent Sandbox Blueprint: Per-Job Pricing ===");
        console.log("Blueprint ID:", blueprintId);
        console.log("Registered jobs:", jobCount);
        console.log("Base rate (token units):", baseRate);
        console.log("Decimals:", decimals);
        console.log("");

        for (uint256 i = 0; i < writeIndexes.length; i++) {
            string memory jobName = _jobName(writeIndexes[i]);
            console.log(
                string.concat(
                    "  Job ", jobName, " (", vm.toString(writeIndexes[i]), "): ", vm.toString(writeRates[i]), " units"
                )
            );
        }

        bool dryRun = vm.envOr("DRY_RUN", false);
        if (!dryRun) {
            vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
            if (setAsset) {
                require(paymentToken != address(0), "SET_ASSET requires PAYMENT_TOKEN");
                tangle.setBlueprintSettlementAsset(blueprintId, paymentToken);
            }
            tangle.setJobEventRates(blueprintId, writeIndexes, writeRates);
            vm.stopBroadcast();

            if (setAsset) {
                require(
                    tangle.getBlueprintSettlementAsset(blueprintId) == paymentToken, "settlement asset was not pinned"
                );
            }
            require(tangle.getJobEventRate(blueprintId, writeIndexes[0]) == writeRates[0], "job rate was not written");
        }

        console.log("");
        console.log(dryRun ? "Dry run. No transaction sent." : "Job rates configured successfully.");
    }

    /// @dev Drop indexes the blueprint never registered. `getDefaultJobRates` is the
    ///      manager's table, not the blueprint's job list.
    function _registeredRates(uint8[] memory jobIndexes, uint256[] memory rates, uint256 jobCount)
        internal
        pure
        returns (uint8[] memory writeIndexes, uint256[] memory writeRates)
    {
        uint256 keep = 0;
        for (uint256 i = 0; i < jobIndexes.length; i++) {
            if (uint256(jobIndexes[i]) < jobCount) keep++;
        }
        writeIndexes = new uint8[](keep);
        writeRates = new uint256[](keep);
        uint256 n = 0;
        for (uint256 i = 0; i < jobIndexes.length; i++) {
            if (uint256(jobIndexes[i]) < jobCount) {
                writeIndexes[n] = jobIndexes[i];
                writeRates[n] = rates[i];
                n++;
            }
        }
    }
}
