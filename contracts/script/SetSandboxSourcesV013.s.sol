// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";

/// @notice Minimal 0.19 interface for repointing a blueprint's cold-start sources
///         and operator ack. Defined locally so the script builds against the
///         soldeer-pinned 0.13 Types (identical BlueprintSource layout) without
///         needing the full 0.19 tnt-core source tree.
interface ITangleSources {
    function setBlueprintSources(uint64 blueprintId, Types.BlueprintSource[] calldata sources) external;
    function ackBlueprintSources(uint64 blueprintId, bytes32 sourcesHash) external;
    function blueprintSourcesHash(uint64 blueprintId) external view returns (bytes32);
    function operatorAckedCurrentSources(uint64 blueprintId, address operator) external view returns (bool);
}

/// @title SetSandboxSourcesV013
/// @notice Repoints blueprint 4 (ai-agent-sandbox) at the v0.1.3 GitHub release
///         carrying the tnt-core-0.19-fixed operator binary, then acks it as the
///         operator so the cold-start manager fetches and verifies the new binary.
/// @dev Two broadcasts: owner sets sources, operator acks. Pass keys via env.
contract SetSandboxSourcesV013 is Script {
    // Blueprint id defaults to 4 (legacy) but is overridable via BLUEPRINT_ID env
    // so the same script can wire the fresh 0.19-BSM blueprint (id 8 on Tempo).
    uint64 immutable BLUEPRINT_ID = uint64(vm.envOr("BLUEPRINT_ID", uint256(4)));

    // v0.1.3 release, inner binary sha256 = the fix (41242ef4…).
    bytes32 constant BIN_SHA = 0x41242ef4a8aa9c3420660b7969eba77b781331a677f035943babe93a60a09cf9;

    string constant ARTIFACT_URI =
        "{\"dist_url\":\"https://github.com/tangle-network/ai-agent-sandbox-blueprint/releases/download/v0.1.3/dist-manifest.json\",\"archive_url\":\"https://github.com/tangle-network/ai-agent-sandbox-blueprint/releases/download/v0.1.3/ai-agent-sandbox-blueprint-x86_64-unknown-linux-gnu.tar.xz\",\"binaries\":[]}";

    function _buildSources() internal pure returns (Types.BlueprintSource[] memory sources) {
        sources = new Types.BlueprintSource[](1);

        Types.BlueprintBinary[] memory bins = new Types.BlueprintBinary[](1);
        bins[0] = Types.BlueprintBinary({
            arch: Types.BlueprintArchitecture.Amd64,
            os: Types.BlueprintOperatingSystem.Linux,
            name: "ai-agent-sandbox-blueprint",
            sha256: BIN_SHA
        });

        sources[0] = Types.BlueprintSource({
            kind: Types.BlueprintSourceKind.Native,
            container: Types.ImageRegistrySource("", "", ""),
            wasm: Types.WasmSource(Types.WasmRuntime.Unknown, Types.BlueprintFetcherKind.None, "", ""),
            native: Types.NativeSource(Types.BlueprintFetcherKind.Http, ARTIFACT_URI, "ai-agent-sandbox-blueprint"),
            testing: Types.TestingSource("", "", ""),
            binaries: bins
        });
    }

    function run() external {
        address tangle = vm.envAddress("TANGLE_CONTRACT");
        uint256 ownerKey = vm.envUint("OWNER_KEY");
        uint256 operatorKey = vm.envUint("OPERATOR_KEY");
        address operator = vm.addr(operatorKey);

        ITangleSources t = ITangleSources(tangle);

        bytes32 before = t.blueprintSourcesHash(BLUEPRINT_ID);
        console.log("sources hash BEFORE:");
        console.logBytes32(before);

        Types.BlueprintSource[] memory sources = _buildSources();
        bytes32 expected = keccak256(abi.encode(sources));
        console.log("expected sources hash (keccak abi.encode):");
        console.logBytes32(expected);

        vm.startBroadcast(ownerKey);
        t.setBlueprintSources(BLUEPRINT_ID, sources);
        vm.stopBroadcast();

        bytes32 live = t.blueprintSourcesHash(BLUEPRINT_ID);
        console.log("sources hash AFTER set:");
        console.logBytes32(live);
        require(live == expected, "live hash != expected (encoding mismatch)");

        vm.startBroadcast(operatorKey);
        t.ackBlueprintSources(BLUEPRINT_ID, live);
        vm.stopBroadcast();

        bool acked = t.operatorAckedCurrentSources(BLUEPRINT_ID, operator);
        console.log("operator acked current sources:", acked);
        require(acked, "operator ack failed");
    }
}
