// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";
import "../src/AgentSandboxBlueprint.sol";
import "../src/AgentInstanceBlueprint.sol";
import "../src/AgentTeeInstanceBlueprint.sol";

/// @notice Minimal interface for Tangle contract blueprint registration
interface ITangle {
    function createBlueprint(Types.BlueprintDefinition calldata def) external returns (uint64);
}

/// @title RegisterBlueprint
/// @notice Deploys all 3 blueprint contracts and registers them on Tangle.
/// @dev Run via: forge script contracts/script/RegisterBlueprint.s.sol --rpc-url $RPC_URL --broadcast --slow
///
///      This script handles the complex BlueprintDefinition struct encoding.
///      Service lifecycle (registerOperator, requestService, approveService) is
///      handled by the bash wrapper (scripts/deploy-local.sh).
contract RegisterBlueprint is Script {
    // Anvil well-known deployer key
    uint256 constant DEPLOYER_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;

    // Tangle protocol address (deterministic from Anvil state snapshot)
    address constant TANGLE = 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9;
    address constant RESTAKING = 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512;

    function run() external {
        ITangle tangle = ITangle(TANGLE);

        vm.startBroadcast(DEPLOYER_KEY);

        // ── Deploy Blueprint Service Managers ────────────────────────────
        AgentSandboxBlueprint sandbox = new AgentSandboxBlueprint(RESTAKING);
        AgentInstanceBlueprint instance = new AgentInstanceBlueprint();
        AgentTeeInstanceBlueprint teeInstance = new AgentTeeInstanceBlueprint();

        // ── Register on Tangle ──────────────────────────────────────────
        uint64 sandboxId = tangle.createBlueprint(_buildSandboxDefinition(address(sandbox)));
        uint64 instanceId = tangle.createBlueprint(_buildInstanceDefinition(address(instance)));
        uint64 teeInstanceId = tangle.createBlueprint(_buildTeeInstanceDefinition(address(teeInstance)));

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

    function _buildSandboxDefinition(address manager)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri = "";
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

        def.metadata = Types.BlueprintMetadata({
            name: "AI Agent Sandbox Blueprint",
            description: "Multi-operator AI sandbox with Docker backends, batch execution, workflows, and SSH access",
            author: "Tangle",
            category: "AI/Compute",
            codeRepository: "https://github.com/tangle-network/ai-agent-sandbox-blueprint",
            logo: "",
            website: "https://tangle.network",
            license: "UNLICENSE",
            profilingData: ""
        });

        // 17 jobs
        def.jobs = new Types.JobDefinition[](17);
        def.jobs[0]  = Types.JobDefinition("sandbox_create", "Create a new AI sandbox", "", "", "");
        def.jobs[1]  = Types.JobDefinition("sandbox_stop", "Stop a running sandbox", "", "", "");
        def.jobs[2]  = Types.JobDefinition("sandbox_resume", "Resume a stopped sandbox", "", "", "");
        def.jobs[3]  = Types.JobDefinition("sandbox_delete", "Delete a sandbox", "", "", "");
        def.jobs[4]  = Types.JobDefinition("sandbox_snapshot", "Snapshot a sandbox", "", "", "");
        def.jobs[5]  = Types.JobDefinition("exec", "Execute shell command", "", "", "");
        def.jobs[6]  = Types.JobDefinition("prompt", "Single LLM inference", "", "", "");
        def.jobs[7]  = Types.JobDefinition("task", "Multi-turn agent task", "", "", "");
        def.jobs[8]  = Types.JobDefinition("batch_create", "Create N sandboxes", "", "", "");
        def.jobs[9]  = Types.JobDefinition("batch_task", "Run tasks on N sandboxes", "", "", "");
        def.jobs[10] = Types.JobDefinition("batch_exec", "Execute on N sandboxes", "", "", "");
        def.jobs[11] = Types.JobDefinition("batch_collect", "Collect batch results", "", "", "");
        def.jobs[12] = Types.JobDefinition("workflow_create", "Create a workflow", "", "", "");
        def.jobs[13] = Types.JobDefinition("workflow_trigger", "Trigger a workflow", "", "", "");
        def.jobs[14] = Types.JobDefinition("workflow_cancel", "Cancel a workflow", "", "", "");
        def.jobs[15] = Types.JobDefinition("ssh_provision", "Grant SSH access", "", "", "");
        def.jobs[16] = Types.JobDefinition("ssh_revoke", "Revoke SSH access", "", "", "");

        def.registrationSchema = "";
        def.requestSchema = "";

        // Native binary source
        def.sources = new Types.BlueprintSource[](1);
        Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
        bins[0] = Types.BlueprintBinary({
            arch: Types.BlueprintArchitecture.Amd64,
            os: Types.BlueprintOperatingSystem.Linux,
            name: "ai-agent-sandbox-blueprint",
            sha256: bytes32(0)
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

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Dynamic;
    }

    function _buildInstanceDefinition(address manager)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri = "";
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

        def.metadata = Types.BlueprintMetadata({
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

        // 8 jobs
        def.jobs = new Types.JobDefinition[](8);
        def.jobs[0] = Types.JobDefinition("provision", "Provision AI sandbox instance", "", "", "");
        def.jobs[1] = Types.JobDefinition("exec", "Execute shell command", "", "", "");
        def.jobs[2] = Types.JobDefinition("prompt", "Single LLM inference", "", "", "");
        def.jobs[3] = Types.JobDefinition("task", "Multi-turn agent task", "", "", "");
        def.jobs[4] = Types.JobDefinition("ssh_provision", "Grant SSH access", "", "", "");
        def.jobs[5] = Types.JobDefinition("ssh_revoke", "Revoke SSH access", "", "", "");
        def.jobs[6] = Types.JobDefinition("snapshot", "Create disk snapshot", "", "", "");
        def.jobs[7] = Types.JobDefinition("deprovision", "Deprovision instance", "", "", "");

        def.registrationSchema = "";
        def.requestSchema = "";

        def.sources = new Types.BlueprintSource[](1);
        Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
        bins[0] = Types.BlueprintBinary({
            arch: Types.BlueprintArchitecture.Amd64,
            os: Types.BlueprintOperatingSystem.Linux,
            name: "ai-agent-instance-blueprint",
            sha256: bytes32(0)
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

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Fixed;
    }

    function _buildTeeInstanceDefinition(address manager)
        internal
        pure
        returns (Types.BlueprintDefinition memory def)
    {
        def.metadataUri = "";
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

        def.metadata = Types.BlueprintMetadata({
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

        // Same 8 jobs as instance
        def.jobs = new Types.JobDefinition[](8);
        def.jobs[0] = Types.JobDefinition("provision", "Provision TEE sandbox instance", "", "", "");
        def.jobs[1] = Types.JobDefinition("exec", "Execute shell command in TEE", "", "", "");
        def.jobs[2] = Types.JobDefinition("prompt", "Single LLM inference in TEE", "", "", "");
        def.jobs[3] = Types.JobDefinition("task", "Multi-turn agent task in TEE", "", "", "");
        def.jobs[4] = Types.JobDefinition("ssh_provision", "Grant SSH access", "", "", "");
        def.jobs[5] = Types.JobDefinition("ssh_revoke", "Revoke SSH access", "", "", "");
        def.jobs[6] = Types.JobDefinition("snapshot", "Create disk snapshot", "", "", "");
        def.jobs[7] = Types.JobDefinition("deprovision", "Deprovision TEE instance", "", "", "");

        def.registrationSchema = "";
        def.requestSchema = "";

        def.sources = new Types.BlueprintSource[](1);
        Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
        bins[0] = Types.BlueprintBinary({
            arch: Types.BlueprintArchitecture.Amd64,
            os: Types.BlueprintOperatingSystem.Linux,
            name: "ai-agent-tee-instance-blueprint",
            sha256: bytes32(0)
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

        def.supportedMemberships = new Types.MembershipModel[](1);
        def.supportedMemberships[0] = Types.MembershipModel.Fixed;
    }
}
