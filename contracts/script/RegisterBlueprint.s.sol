// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";
import "../src/AgentSandboxBlueprint.sol";

/// @notice Minimal interface for Tangle contract blueprint registration
interface ITangle {
    function createBlueprint(Types.BlueprintDefinition calldata def) external returns (uint64);
}

/// @title RegisterBlueprint
/// @notice Deploys 1 unified contract 3 times with different mode flags and registers on Tangle.
/// @dev Run via: forge script contracts/script/RegisterBlueprint.s.sol --rpc-url $RPC_URL --broadcast --slow
contract RegisterBlueprint is Script {
    // Anvil well-known deployer key (default when no PRIVATE_KEY env is set)
    uint256 constant DEFAULT_DEPLOYER_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;

    // Tangle protocol addresses on a LocalTestnet anvil snapshot.
    // For real chains (Base Sepolia, mainnet) pass via env: TANGLE_CORE, RESTAKING.
    address constant DEFAULT_TANGLE = 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9;
    address constant DEFAULT_RESTAKING = 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512;

    function run() external {
        uint256 deployerKey = vm.envOr("PRIVATE_KEY", DEFAULT_DEPLOYER_KEY);
        address tangleAddr = vm.envOr("TANGLE_CORE", DEFAULT_TANGLE);
        address restakingAddr = vm.envOr("RESTAKING", DEFAULT_RESTAKING);

        // Some chains (e.g. Tempo) meter execution far above eth_estimateGas AND
        // enforce a per-tx gas cap (Tempo: 30M). createBlueprint's cost is
        // dominated by the master blueprint service manager re-encoding the full
        // definition; the full ~10.6KB definition pushes it past 30M there.
        // COMPACT_DEFINITION trims the definition to the driver-essential jobs
        // (sandbox_create/delete) with empty descriptions and minimal metadata,
        // which halves the encoded size and brings the call under the cap. The
        // job *names/indices* the driver relies on are preserved.
        bool compact = vm.envOr("COMPACT_DEFINITION", false);

        ITangle tangle = ITangle(tangleAddr);

        vm.startBroadcast(deployerKey);

        // ── Blueprint Service Managers ───────────────────────────────────
        // Reuse already-deployed managers when their addresses are supplied
        // (SANDBOX_BSM / INSTANCE_BSM / TEE_INSTANCE_BSM). On chains that
        // enforce a per-tx gas cap (e.g. Tempo's 30M), a manager deploy (~18M)
        // and createBlueprint (~7.5M) cannot share one gas-estimate multiplier
        // without one of them exceeding the cap or running out of gas. Splitting
        // the two phases across runs lets each pick a safe multiplier: deploy
        // the managers once, then re-run with the addresses to register only.
        address sandboxBsm = vm.envOr("SANDBOX_BSM", address(0));
        address instanceBsm = vm.envOr("INSTANCE_BSM", address(0));
        address teeBsm = vm.envOr("TEE_INSTANCE_BSM", address(0));

        // Cloud mode: capacity-weighted operator selection
        AgentSandboxBlueprint sandbox = sandboxBsm != address(0)
            ? AgentSandboxBlueprint(payable(sandboxBsm))
            : new AgentSandboxBlueprint(restakingAddr, false, false);
        // Instance mode: per-service singleton sandbox
        AgentSandboxBlueprint instance = instanceBsm != address(0)
            ? AgentSandboxBlueprint(payable(instanceBsm))
            : new AgentSandboxBlueprint(address(0), true, false);
        // TEE instance mode: singleton with attestation enforcement
        AgentSandboxBlueprint teeInstance = teeBsm != address(0)
            ? AgentSandboxBlueprint(payable(teeBsm))
            : new AgentSandboxBlueprint(address(0), true, true);

        // ── Register on Tangle ──────────────────────────────────────────
        uint64 sandboxId = tangle.createBlueprint(_buildSandboxDefinition(address(sandbox), compact));
        uint64 instanceId = tangle.createBlueprint(_buildInstanceDefinition(address(instance), compact));
        uint64 teeInstanceId = tangle.createBlueprint(_buildTeeInstanceDefinition(address(teeInstance), compact));

        vm.stopBroadcast();

        // ── Output for bash wrapper parsing ─────────────────────────────
        console.log("DEPLOY_SANDBOX_BSM=%s", vm.toString(address(sandbox)));
        console.log("DEPLOY_INSTANCE_BSM=%s", vm.toString(address(instance)));
        console.log("DEPLOY_TEE_INSTANCE_BSM=%s", vm.toString(address(teeInstance)));
        console.log("DEPLOY_SANDBOX_BLUEPRINT_ID=%s", vm.toString(sandboxId));
        console.log("DEPLOY_INSTANCE_BLUEPRINT_ID=%s", vm.toString(instanceId));
        console.log("DEPLOY_TEE_INSTANCE_BLUEPRINT_ID=%s", vm.toString(teeInstanceId));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Blueprint Definition builders
    // ═════════════════════════════════════════════════════════════════════════

    function _buildCloudJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](5);
        jobs[0] = Types.JobDefinition("sandbox_create", "Create a new AI sandbox", "", "", "");
        jobs[1] = Types.JobDefinition("sandbox_delete", "Delete an AI sandbox", "", "", "");
        jobs[2] = Types.JobDefinition("workflow_create", "Create or update a workflow", "", "", "");
        jobs[3] = Types.JobDefinition("workflow_trigger", "Trigger a workflow execution", "", "", "");
        jobs[4] = Types.JobDefinition("workflow_cancel", "Cancel an active workflow", "", "", "");
    }

    function _buildInstanceJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](5);
        // IDs are positional in Tangle metadata. Keep 0/1 reserved so workflow IDs stay 2/3/4.
        jobs[0] = Types.JobDefinition(
            "cloud_only_reserved_sandbox_create", "Reserved in instance mode (cloud sandbox lifecycle only)", "", "", ""
        );
        jobs[1] = Types.JobDefinition(
            "cloud_only_reserved_sandbox_delete", "Reserved in instance mode (cloud sandbox lifecycle only)", "", "", ""
        );
        jobs[2] = Types.JobDefinition("workflow_create", "Create or update a workflow", "", "", "");
        jobs[3] = Types.JobDefinition("workflow_trigger", "Trigger a workflow execution", "", "", "");
        jobs[4] = Types.JobDefinition("workflow_cancel", "Cancel an active workflow", "", "", "");
    }

    /// @dev Gas-lean job set: the two lifecycle jobs the driver submits (indices
    /// 0/1, sandbox_create/delete) with empty descriptions. Trims the definition
    /// the master manager stores on-chain for chains that meter storage above
    /// standard while preserving the driver's job-index contract.
    function _compactJobs() internal pure returns (Types.JobDefinition[] memory jobs) {
        jobs = new Types.JobDefinition[](2);
        jobs[0] = Types.JobDefinition("sandbox_create", "", "", "", "");
        jobs[1] = Types.JobDefinition("sandbox_delete", "", "", "", "");
    }

    /// @dev Minimal metadata (name only) for gas-cap-constrained chains.
    function _compactMetadata(string memory name) internal pure returns (Types.BlueprintMetadata memory) {
        return Types.BlueprintMetadata(name, "", "", "", "", "", "", "", "");
    }

    /// @dev Minimal single native source (createBlueprint requires >=1 source;
    /// an empty array reverts). Short strings keep the definition the master
    /// manager stores on-chain as small as possible for gas-cap-constrained chains.
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
        def.metadataUri = compact ? "tangle:ai-agent-sandbox" : "https://github.com/tangle-network/ai-agent-sandbox-blueprint";
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
            eventRate: 1e15 // 0.001 TNT base rate
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

        if (compact) {
            def.sources = _compactSources();
        } else {
            def.sources = new Types.BlueprintSource[](1);
            Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
            bins[0] = Types.BlueprintBinary({
                arch: Types.BlueprintArchitecture.Amd64,
                os: Types.BlueprintOperatingSystem.Linux,
                name: "ai-agent-sandbox-blueprint",
                sha256: bytes32(uint256(0xdeadbeef))
            });
            def.sources[0] = Types.BlueprintSource({
                kind: Types.BlueprintSourceKind.Native,
                container: Types.ImageRegistrySource("", "", ""),
                wasm: Types.WasmSource(Types.WasmRuntime.Unknown, Types.BlueprintFetcherKind.None, "", ""),
                native: Types.NativeSource(
                    Types.BlueprintFetcherKind.None,
                    "file:///target/release/ai-agent-sandbox-blueprint-bin",
                    "./target/release/ai-agent-sandbox-blueprint-bin"
                ),
                testing: Types.TestingSource("ai-agent-sandbox-blueprint-bin", "ai-agent-sandbox-blueprint", "."),
                binaries: bins
            });
        }

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Dynamic;
    }

    function _buildInstanceDefinition(address manager, bool compact)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri = compact ? "tangle:ai-agent-instance" : "https://github.com/tangle-network/ai-agent-sandbox-blueprint";
        def.metadataHash = keccak256(bytes(def.metadataUri));
        def.manager = manager;
        def.masterManagerRevision = 0;
        def.hasConfig = true;

        def.config = Types.BlueprintConfig({
            membership: Types.MembershipModel.Fixed,
            pricing: Types.PricingModel.Subscription,
            minOperators: 1,
            maxOperators: 10,
            subscriptionRate: 1e16, // 0.01 TNT per interval
            subscriptionInterval: 86400, // daily
            eventRate: 0
        });

        def.metadata = compact
            ? _compactMetadata("AI Agent Instance Blueprint")
            : Types.BlueprintMetadata({
            name: "AI Agent Instance Blueprint",
            description: "Subscription-based replicated AI sandbox with multi-operator redundancy",
            author: "Tangle",
            category: "AI/Compute",
            codeRepository: "https://github.com/tangle-network/ai-agent-sandbox-blueprint",
            logo: "",
            website: "https://tangle.network",
            license: "UNLICENSE",
            profilingData: ""
        });

        def.jobs = compact ? _compactJobs() : _buildInstanceJobs();

        def.registrationSchema = "";
        def.requestSchema = "";

        if (compact) {
            def.sources = _compactSources();
        } else {
            def.sources = new Types.BlueprintSource[](1);
            Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
            bins[0] = Types.BlueprintBinary({
                arch: Types.BlueprintArchitecture.Amd64,
                os: Types.BlueprintOperatingSystem.Linux,
                name: "ai-agent-instance-blueprint",
                sha256: bytes32(uint256(0xdeadbeef))
            });
            def.sources[0] = Types.BlueprintSource({
                kind: Types.BlueprintSourceKind.Native,
                container: Types.ImageRegistrySource("", "", ""),
                wasm: Types.WasmSource(Types.WasmRuntime.Unknown, Types.BlueprintFetcherKind.None, "", ""),
                native: Types.NativeSource(
                    Types.BlueprintFetcherKind.None,
                    "file:///target/release/ai-agent-instance-blueprint-bin",
                    "./target/release/ai-agent-instance-blueprint-bin"
                ),
                testing: Types.TestingSource("ai-agent-instance-blueprint-bin", "ai-agent-instance-blueprint", "."),
                binaries: bins
            });
        }

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Fixed;
    }

    function _buildTeeInstanceDefinition(address manager, bool compact)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri = compact ? "tangle:ai-agent-tee" : "https://github.com/tangle-network/ai-agent-sandbox-blueprint";
        def.metadataHash = keccak256(bytes(def.metadataUri));
        def.manager = manager;
        def.masterManagerRevision = 0;
        def.hasConfig = true;

        def.config = Types.BlueprintConfig({
            membership: Types.MembershipModel.Fixed,
            pricing: Types.PricingModel.Subscription,
            minOperators: 1,
            maxOperators: 10,
            subscriptionRate: 5e16, // 0.05 TNT per interval (TEE premium)
            subscriptionInterval: 86400,
            eventRate: 0
        });

        def.metadata = compact
            ? _compactMetadata("AI Agent TEE Instance Blueprint")
            : Types.BlueprintMetadata({
            name: "AI Agent TEE Instance Blueprint",
            description: "TEE-backed replicated AI sandbox with attestation verification",
            author: "Tangle",
            category: "AI/Compute",
            codeRepository: "https://github.com/tangle-network/ai-agent-sandbox-blueprint",
            logo: "",
            website: "https://tangle.network",
            license: "UNLICENSE",
            profilingData: ""
        });

        def.jobs = compact ? _compactJobs() : _buildInstanceJobs();

        def.registrationSchema = "";
        def.requestSchema = "";

        if (compact) {
            def.sources = _compactSources();
        } else {
            def.sources = new Types.BlueprintSource[](1);
            Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
            bins[0] = Types.BlueprintBinary({
                arch: Types.BlueprintArchitecture.Amd64,
                os: Types.BlueprintOperatingSystem.Linux,
                name: "ai-agent-tee-instance-blueprint",
                sha256: bytes32(uint256(0xdeadbeef))
            });
            def.sources[0] = Types.BlueprintSource({
                kind: Types.BlueprintSourceKind.Native,
                container: Types.ImageRegistrySource("", "", ""),
                wasm: Types.WasmSource(Types.WasmRuntime.Unknown, Types.BlueprintFetcherKind.None, "", ""),
                native: Types.NativeSource(
                    Types.BlueprintFetcherKind.None,
                    "file:///target/release/ai-agent-tee-instance-blueprint-bin",
                    "./target/release/ai-agent-tee-instance-blueprint-bin"
                ),
                testing: Types.TestingSource("ai-agent-tee-instance-blueprint-bin", "ai-agent-tee-instance-blueprint", "."),
                binaries: bins
            });
        }

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Fixed;
    }
}
