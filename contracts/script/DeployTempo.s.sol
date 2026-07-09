// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";
import "../src/AgentSandboxBlueprint.sol";

/// @notice Minimal interface for Tangle contract blueprint registration.
interface ITangle {
    function createBlueprint(Types.BlueprintDefinition calldata def) external returns (uint64);
    function blueprintCount() external view returns (uint64);
}

/// @title DeployTempo
/// @notice Deploys ONE fresh cloud-mode `AgentSandboxBlueprint` (compiled against
///         tnt-core 0.19 — onJobResult selector 0xe649fc03) UNBOUND and registers
///         exactly one sandbox blueprint on the live Tempo Tangle. The manager
///         binds to this blueprint during `createBlueprint`; do NOT pre-call
///         onBlueprintCreated (it reverts AlreadyInitialized).
///
/// Deploy (Tempo, 30M per-tx gas cap):
///   PRIVATE_KEY=<deployer> TANGLE_CORE=0xff137b9c879c47c28ce389e84501925438ab4cda \
///   RESTAKING=0x9484d07899b98384f1d66bd5b2659f3ed346f89e \
///   forge script contracts/script/DeployTempo.s.sol:DeployTempo \
///     --rpc-url https://rpc.moderato.tempo.xyz --broadcast --slow \
///     --gas-estimate-multiplier 110 --skip test
contract DeployTempo is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address tangleAddr = vm.envAddress("TANGLE_CORE");
        address restakingAddr = vm.envAddress("RESTAKING");
        // Trim the on-chain-stored definition to fit the Tempo 30M per-tx cap.
        bool compact = vm.envOr("COMPACT_DEFINITION", true);

        ITangle tangle = ITangle(tangleAddr);
        uint64 countBefore = tangle.blueprintCount();
        console.log("blueprintCount BEFORE:", countBefore);

        vm.startBroadcast(deployerKey);

        // Cloud mode: capacity-weighted operator selection. UNBOUND — the master
        // manager binds it below during createBlueprint.
        AgentSandboxBlueprint sandbox = new AgentSandboxBlueprint(restakingAddr, false, false);

        uint64 blueprintId = tangle.createBlueprint(_buildSandboxDefinition(address(sandbox), compact));

        vm.stopBroadcast();

        console.log("DEPLOY_SANDBOX_BSM=%s", vm.toString(address(sandbox)));
        console.log("DEPLOY_SANDBOX_BLUEPRINT_ID=%s", vm.toString(blueprintId));
        console.log("blueprintCount AFTER:", tangle.blueprintCount());
        require(blueprintId == countBefore, "unexpected blueprint id");
    }

    function _buildCloudJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](5);
        jobs[0] = Types.JobDefinition("sandbox_create", "Create a new AI sandbox", "", "", "");
        jobs[1] = Types.JobDefinition("sandbox_delete", "Delete an AI sandbox", "", "", "");
        jobs[2] = Types.JobDefinition("workflow_create", "Create or update a workflow", "", "", "");
        jobs[3] = Types.JobDefinition("workflow_trigger", "Trigger a workflow execution", "", "", "");
        jobs[4] = Types.JobDefinition("workflow_cancel", "Cancel an active workflow", "", "", "");
    }

    /// @dev Gas-lean job set: the two lifecycle jobs the driver submits (indices
    /// 0/1, sandbox_create/delete) with empty descriptions. Preserves the
    /// driver's job-index contract while shrinking the stored definition.
    function _compactJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](2);
        jobs[0] = Types.JobDefinition("sandbox_create", "", "", "", "");
        jobs[1] = Types.JobDefinition("sandbox_delete", "", "", "", "");
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

    function _buildSandboxDefinition(address manager, bool compact)
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
            eventRate: 1e15 // 0.001 TNT base rate; per-job PathUSD rates set post-registration
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
