// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";
import "../src/AgentSandboxBlueprint.sol";

/// @notice Minimal interface for Tangle contract blueprint registration
interface ITangle {
    function createBlueprint(Types.BlueprintDefinition calldata def) external returns (uint64);
}

address constant TEMPO_TANGLE = 0xff137B9C879c47C28ce389e84501925438AB4cdA;

/// @title RegisterBlueprint
/// @notice Deploys 1 unified contract 3 times with different mode flags and registers on Tangle.
/// @dev Run via: forge script contracts/script/RegisterBlueprint.s.sol --rpc-url $RPC_URL --broadcast --slow
contract RegisterBlueprint is Script {
    // Anvil well-known deployer key (default when no PRIVATE_KEY env is set)
    uint256 constant DEFAULT_DEPLOYER_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;

    // Tangle protocol addresses on a LocalTestnet anvil snapshot.
    // For real chains (Base Sepolia, mainnet) pass via env: TANGLE_CORE, RESTAKING.
    address constant DEFAULT_TANGLE = 0x0165878A594ca255338adfa4d48449f69242Eb8F;
    address constant DEFAULT_RESTAKING = 0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9;

    function run() external {
        uint256 deployerKey = vm.envOr("PRIVATE_KEY", DEFAULT_DEPLOYER_KEY);
        address tangleAddr = vm.envOr("TANGLE_CORE", DEFAULT_TANGLE);
        address restakingAddr = vm.envOr("RESTAKING", DEFAULT_RESTAKING);
        // Tempo pathUSD is 6 decimals. This script's unset rate is 1e15, which
        // is $1B of pathUSD. DeployTempo pins pathUSD and writes the 0.001 base.
        require(tangleAddr != TEMPO_TANGLE, "Tempo: use contracts/script/DeployTempo.s.sol");

        // COMPACT_DEFINITION keeps sandbox_create/delete and drops the rest so
        // createBlueprint fits Tempo's 30M per-tx cap. Tempo pathUSD uses DeployTempo.
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
            : new AgentSandboxBlueprint(restakingAddr, false, false, address(0));
        // Instance mode: per-service singleton sandbox
        AgentSandboxBlueprint instance = instanceBsm != address(0)
            ? AgentSandboxBlueprint(payable(instanceBsm))
            : new AgentSandboxBlueprint(address(0), true, false, address(0));
        // TEE instance mode: singleton with attestation enforcement
        AgentSandboxBlueprint teeInstance = teeBsm != address(0)
            ? AgentSandboxBlueprint(payable(teeBsm))
            : new AgentSandboxBlueprint(address(0), true, true, address(0));

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

    struct _DefSpec {
        string compactUri;
        string name;
        string description;
        Types.MembershipModel membership;
        Types.PricingModel pricing;
        uint32 maxOperators;
        uint256 subscriptionRate;
        uint64 subscriptionInterval;
        uint256 eventRate;
        bool instanceJobs;
        string binName;
        string nativeFetcher;
        string nativePath;
        string testBin;
        string testName;
    }

    function _nativeSources(
        string memory binName,
        string memory nativeFetcher,
        string memory nativePath,
        string memory testBin,
        string memory testName
    ) internal pure returns (Types.BlueprintSource[] memory sources) {
        sources = new Types.BlueprintSource[](1);
        Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
        bins[0] = Types.BlueprintBinary({
            arch: Types.BlueprintArchitecture.Amd64,
            os: Types.BlueprintOperatingSystem.Linux,
            name: binName,
            sha256: bytes32(uint256(0xdeadbeef))
        });
        sources[0] = Types.BlueprintSource({
            kind: Types.BlueprintSourceKind.Native,
            container: Types.ImageRegistrySource("", "", ""),
            wasm: Types.WasmSource(Types.WasmRuntime.Unknown, Types.BlueprintFetcherKind.None, "", ""),
            native: Types.NativeSource(Types.BlueprintFetcherKind.None, nativeFetcher, nativePath),
            testing: Types.TestingSource(testBin, testName, "."),
            binaries: bins
        });
    }

    function _definition(address manager, bool compact, _DefSpec memory spec)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri = compact ? spec.compactUri : "https://github.com/tangle-network/ai-agent-sandbox-blueprint";
        def.metadataHash = keccak256(bytes(def.metadataUri));
        def.manager = manager;
        def.masterManagerRevision = 0;
        def.hasConfig = true;
        def.config = Types.BlueprintConfig({
            membership: spec.membership,
            pricing: spec.pricing,
            minOperators: 1,
            maxOperators: spec.maxOperators,
            subscriptionRate: spec.subscriptionRate,
            subscriptionInterval: spec.subscriptionInterval,
            eventRate: spec.eventRate
        });
        def.metadata = compact
            ? _compactMetadata(spec.name)
            : Types.BlueprintMetadata({
                name: spec.name,
                description: spec.description,
                author: "Tangle",
                category: "AI/Compute",
                codeRepository: "https://github.com/tangle-network/ai-agent-sandbox-blueprint",
                logo: "",
                website: "https://tangle.network",
                license: "UNLICENSE",
                profilingData: ""
            });
        def.jobs = compact ? _compactJobs() : (spec.instanceJobs ? _buildInstanceJobs() : _buildCloudJobs());
        def.registrationSchema = "";
        def.requestSchema = "";
        def.sources = compact
            ? _compactSources()
            : _nativeSources(spec.binName, spec.nativeFetcher, spec.nativePath, spec.testBin, spec.testName);
        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = spec.membership;
    }

    function _buildSandboxDefinition(address manager, bool compact)
        internal
        pure
        returns (Types.BlueprintDefinition memory)
    {
        return _definition(
            manager,
            compact,
            _DefSpec({
                compactUri: "tangle:ai-agent-sandbox",
                name: "AI Agent Sandbox Blueprint",
                description: "Multi-operator AI sandbox with Docker backends, workflows, and SSH access",
                membership: Types.MembershipModel.Dynamic,
                pricing: Types.PricingModel.EventDriven,
                maxOperators: 100,
                subscriptionRate: 0,
                subscriptionInterval: 0,
                eventRate: 1e15,
                instanceJobs: false,
                binName: "ai-agent-sandbox-blueprint",
                nativeFetcher: "file:///target/release/ai-agent-sandbox-blueprint-bin",
                nativePath: "./target/release/ai-agent-sandbox-blueprint-bin",
                testBin: "ai-agent-sandbox-blueprint-bin",
                testName: "ai-agent-sandbox-blueprint"
            })
        );
    }

    function _buildInstanceDefinition(address manager, bool compact)
        internal
        pure
        returns (Types.BlueprintDefinition memory)
    {
        return _definition(
            manager,
            compact,
            _DefSpec({
                compactUri: "tangle:ai-agent-instance",
                name: "AI Agent Instance Blueprint",
                description: "Subscription-based replicated AI sandbox with multi-operator redundancy",
                membership: Types.MembershipModel.Fixed,
                pricing: Types.PricingModel.Subscription,
                maxOperators: 10,
                subscriptionRate: 1e16,
                subscriptionInterval: 86400,
                eventRate: 0,
                instanceJobs: true,
                binName: "ai-agent-instance-blueprint",
                nativeFetcher: "file:///target/release/ai-agent-instance-blueprint-bin",
                nativePath: "./target/release/ai-agent-instance-blueprint-bin",
                testBin: "ai-agent-instance-blueprint-bin",
                testName: "ai-agent-instance-blueprint"
            })
        );
    }

    function _buildTeeInstanceDefinition(address manager, bool compact)
        internal
        pure
        returns (Types.BlueprintDefinition memory)
    {
        return _definition(
            manager,
            compact,
            _DefSpec({
                compactUri: "tangle:ai-agent-tee",
                name: "AI Agent TEE Instance Blueprint",
                description: "TEE-backed replicated AI sandbox with attestation verification",
                membership: Types.MembershipModel.Fixed,
                pricing: Types.PricingModel.Subscription,
                maxOperators: 10,
                subscriptionRate: 5e16,
                subscriptionInterval: 86400,
                eventRate: 0,
                instanceJobs: true,
                binName: "ai-agent-tee-instance-blueprint",
                nativeFetcher: "file:///target/release/ai-agent-tee-instance-blueprint-bin",
                nativePath: "./target/release/ai-agent-tee-instance-blueprint-bin",
                testBin: "ai-agent-tee-instance-blueprint-bin",
                testName: "ai-agent-tee-instance-blueprint"
            })
        );
    }
}
