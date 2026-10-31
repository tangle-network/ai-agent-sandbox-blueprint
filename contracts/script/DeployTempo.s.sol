// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";
import "../src/AgentSandboxBlueprint.sol";

interface IERC20Decimals {
    function decimals() external view returns (uint8);
}

/// @notice Tempo Moderato diamond. Prices are configuration on this contract.
///         A new diamond is not required to change a rate.
interface ITangle {
    function createBlueprint(Types.BlueprintDefinition calldata def) external returns (uint64);
    function blueprintCount() external view returns (uint64);
    function setBlueprintSettlementAsset(uint64 blueprintId, address asset) external;
    function setJobEventRates(uint64 blueprintId, uint8[] calldata jobIndexes, uint256[] calldata rates) external;
    function getBlueprintSettlementAsset(uint64 blueprintId) external view returns (address);
    function getJobEventRate(uint64 blueprintId, uint8 jobIndex) external view returns (uint256);
}

/// @notice Deploys one unbound cloud manager and registers it on Tempo Moderato.
///         eventRate is the 1× base (`defaultBaseForDecimals`, pathUSD = 1000).
///         Not a diamond redeploy and not mainnet. BASE_RATE overrides the base.
contract DeployTempo is Script {
    address constant PATH_USD = 0x20C0000000000000000000000000000000000000;

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address tangleAddr = vm.envAddress("TANGLE_CORE");
        address restakingAddr = vm.envAddress("RESTAKING");
        address paymentToken = vm.envOr("PAYMENT_TOKEN", PATH_USD);
        // Trim stored strings to fit the Tempo 30M per-tx cap. Job names and
        // indexes 0–4 stay, so the driver's create/delete contract is unchanged.
        bool compact = vm.envOr("COMPACT_DEFINITION", true);

        uint8 decimals = IERC20Decimals(paymentToken).decimals();
        ITangle tangle = ITangle(tangleAddr);
        uint64 countBefore = tangle.blueprintCount();
        console.log("blueprintCount BEFORE:", countBefore);
        console.log("payment decimals:", decimals);

        vm.startBroadcast(deployerKey);

        // Cloud mode: capacity-weighted operator selection. UNBOUND — the master
        // manager binds it during createBlueprint.
        AgentSandboxBlueprint sandbox = new AgentSandboxBlueprint(restakingAddr, false, false, address(0));
        uint256 base = vm.envOr("BASE_RATE", sandbox.defaultBaseForDecimals(decimals));
        uint256 maxBase = sandbox.maxBaseForDecimals(decimals);
        require(base > 0 && base <= maxBase, "BASE_RATE out of range for settlement decimals");

        uint64 blueprintId = tangle.createBlueprint(_buildSandboxDefinition(address(sandbox), compact, base));
        tangle.setBlueprintSettlementAsset(blueprintId, paymentToken);
        (uint8[] memory jobIndexes, uint256[] memory rates) = sandbox.getDefaultJobRates(base);
        tangle.setJobEventRates(blueprintId, jobIndexes, rates);

        vm.stopBroadcast();

        console.log("DEPLOY_SANDBOX_BSM=%s", vm.toString(address(sandbox)));
        console.log("DEPLOY_SANDBOX_BLUEPRINT_ID=%s", vm.toString(blueprintId));
        console.log("base (token units):", base);
        console.log("blueprintCount AFTER:", tangle.blueprintCount());
        require(blueprintId == countBefore, "unexpected blueprint id");
        require(tangle.getBlueprintSettlementAsset(blueprintId) == paymentToken, "settlement asset was not pinned");
        require(tangle.getJobEventRate(blueprintId, 0) == base * 50, "sandbox_create rate");
        require(tangle.getJobEventRate(blueprintId, 1) == base, "sandbox_delete rate");
    }

    function _buildCloudJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](5);
        jobs[0] = Types.JobDefinition("sandbox_create", "Create a new AI sandbox", "", "", "");
        jobs[1] = Types.JobDefinition("sandbox_delete", "Delete an AI sandbox", "", "", "");
        jobs[2] = Types.JobDefinition("workflow_create", "Create or update a workflow", "", "", "");
        jobs[3] = Types.JobDefinition("workflow_trigger", "Trigger a workflow execution", "", "", "");
        jobs[4] = Types.JobDefinition("workflow_cancel", "Cancel an active workflow", "", "", "");
    }

    /// @dev Gas-lean job set: all five settled jobs, empty descriptions.
    ///      Indexes match getDefaultJobRates. Dropping 2–4 is what left those
    ///      jobs on the eventRate fallback.
    function _compactJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](5);
        jobs[0] = Types.JobDefinition("sandbox_create", "", "", "", "");
        jobs[1] = Types.JobDefinition("sandbox_delete", "", "", "", "");
        jobs[2] = Types.JobDefinition("workflow_create", "", "", "", "");
        jobs[3] = Types.JobDefinition("workflow_trigger", "", "", "", "");
        jobs[4] = Types.JobDefinition("workflow_cancel", "", "", "", "");
    }

    function _compactMetadata(string memory name) internal pure returns (Types.BlueprintMetadata memory) {
        return Types.BlueprintMetadata(name, "", "", "", "", "", "", "", "");
    }

    /// @dev createBlueprint requires >=1 source; the real binary source is wired
    /// post-registration via setBlueprintSources (event-sourced). A minimal
    /// placeholder source keeps the definition small under the gas cap.
    function _compactSources() internal pure returns (Types.BlueprintSource[] memory sources) {
        sources = new Types.BlueprintSource[](1);
        Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
        bins[0] = Types.BlueprintBinary({
            arch: Types.BlueprintArchitecture.Amd64,
            os: Types.BlueprintOperatingSystem.Linux,
            name: "bin",
            sha256: bytes32(uint256(0xdeadbeef))
        });
        sources[0] = Types.BlueprintSource({
            kind: Types.BlueprintSourceKind.Native,
            container: Types.ImageRegistrySource("", "", ""),
            wasm: Types.WasmSource(Types.WasmRuntime.Unknown, Types.BlueprintFetcherKind.None, "", ""),
            native: Types.NativeSource(Types.BlueprintFetcherKind.None, "bin", "bin"),
            testing: Types.TestingSource("bin", "bin", "."),
            binaries: bins
        });
    }

    function _buildSandboxDefinition(address manager, bool compact, uint256 eventRate)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri =
            compact ? "tangle:ai-agent-sandbox" : "https://github.com/tangle-network/ai-agent-sandbox-blueprint";
        def.metadataHash = keccak256(bytes(def.metadataUri));
        def.manager = manager;
        def.masterManagerRevision = 0;
        def.hasConfig = true;

        def.config = Types.BlueprintConfig({
            membership: Types.MembershipModel.Dynamic,
            pricing: Types.PricingModel.EventDriven,
            minOperators: 1,
            maxOperators: 100,
            subscriptionRate: 0,
            subscriptionInterval: 0,
            eventRate: eventRate // 1x base, in the settlement token's decimals
        });

        def.metadata = compact
            ? _compactMetadata("AI Agent Sandbox Blueprint")
            : Types.BlueprintMetadata({
                name: "AI Agent Sandbox Blueprint",
                description: "Multi-operator AI sandbox with Docker backends, workflows, and SSH access",
                author: "Tangle",
                category: "AI/Compute",
                codeRepository: "https://github.com/tangle-network/ai-agent-sandbox-blueprint",
                logo: "",
                website: "https://tangle.network",
                license: "UNLICENSE",
                profilingData: ""
            });

        def.jobs = compact ? _compactJobs() : _buildCloudJobs();
        def.registrationSchema = "";
        def.requestSchema = "";
        def.sources = _compactSources();

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Dynamic;
    }
}
