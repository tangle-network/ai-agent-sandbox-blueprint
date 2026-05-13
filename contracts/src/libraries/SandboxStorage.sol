// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "tnt-core/interfaces/IMultiAssetDelegation.sol";
import "./SandboxTypes.sol";

/// @title SandboxStorage
/// @notice ERC-7201 namespaced storage layout for `AgentSandboxBlueprint`.
///
/// Splitting the blueprint into a thin entry-point contract plus a set of
/// external libraries (one DELEGATECALL hop apart) only works if every actor
/// agrees on the storage layout. Putting all mutable state behind a single
/// struct anchored at a deterministic slot means the blueprint and every
/// library see the same fields at the same offsets — no per-function
/// storage-ref threading, no silently-shifting slot indices when fields are
/// added or reordered.
///
/// The slot is derived per ERC-7201:
///   keccak256(abi.encode(uint256(keccak256("tangle.sandbox.blueprint.main")) - 1))
///   & ~bytes32(uint256(0xff))
///
/// Recompute via `cast index uint256 $(cast keccak "tangle.sandbox.blueprint.main") - 1`
/// then bitwise-and with `~0xff` if the constant ever needs to change.
library SandboxStorage {
    /// @dev Single source of truth for blueprint state. Order is locked-in
    /// once the contract ships — append-only.
    struct Data {
        // ── Mode flags ───────────────────────────────────────────────────
        bool instanceMode;
        bool teeRequired;
        // ── Cloud-mode capacity tracking ─────────────────────────────────
        mapping(address => uint32) operatorMaxCapacity;
        mapping(address => uint32) operatorActiveSandboxes;
        uint32 defaultMaxCapacity;
        uint32 totalActiveSandboxes;
        // ── Cloud-mode operator assignment + sandbox registry ────────────
        mapping(uint64 => mapping(uint64 => address)) createAssignments;
        uint256 selectionNonce;
        mapping(bytes32 => address) sandboxOperator;
        mapping(bytes32 => bool) sandboxActive;
        // ── Workflow state ───────────────────────────────────────────────
        mapping(uint64 => SandboxTypes.WorkflowConfig) workflows;
        mapping(uint64 => uint256) workflowIndex;
        uint64[] workflowIds;
        // ── Instance-mode state ──────────────────────────────────────────
        mapping(uint64 => uint32) instanceOperatorCount;
        mapping(uint64 => mapping(address => bool)) operatorProvisioned;
        mapping(uint64 => mapping(address => bytes32)) operatorAttestationHash;
        mapping(uint64 => address[]) serviceOperators;
        mapping(uint64 => mapping(address => uint256)) operatorIndex;
        mapping(uint64 => mapping(address => string)) operatorSidecarUrl;
        uint256 totalProvisionedOperators;
        // ── Per-service config ───────────────────────────────────────────
        mapping(uint64 => bytes) pendingRequestConfig;
        mapping(uint64 => bytes) serviceConfig;
        mapping(uint64 => address) serviceOwner;
    }

    /// keccak256(abi.encode(uint256(keccak256("tangle.sandbox.blueprint.main")) - 1)) & ~bytes32(uint256(0xff))
    bytes32 private constant STORAGE_LOCATION = 0x7570a0aa20487165d9e428dadee7d2c71adbabed29ef953f043f08164618cb00;

    function load() internal pure returns (Data storage $) {
        bytes32 slot = STORAGE_LOCATION;
        assembly {
            $.slot := slot
        }
    }
}
