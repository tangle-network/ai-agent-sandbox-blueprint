// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

/// @title SandboxTypes
/// @notice Structs, constants, events, and errors shared between
///         `AgentSandboxBlueprint` and every helper library. Declared as
///         a library so static members resolve from every external-library
///         callsite without an instance. The blueprint contract keeps the
///         pre-split public ABI by re-emitting events with the same
///         signatures.
library SandboxTypes {
    // ═══════════════════════════════════════════════════════════════════════════
    // CAPS
    // ═══════════════════════════════════════════════════════════════════════════

    uint256 internal constant MAX_WORKFLOWS = 10000;
    uint32 internal constant MAX_OPERATORS_PER_SERVICE = 1000;
    uint256 internal constant MAX_SANDBOX_ID_LENGTH = 255;

    uint8 internal constant WORKFLOW_TARGET_SANDBOX = 0;
    uint8 internal constant WORKFLOW_TARGET_INSTANCE = 1;

    // ═══════════════════════════════════════════════════════════════════════════
    // STRUCTS
    // ═══════════════════════════════════════════════════════════════════════════

    struct WorkflowCreateRequest {
        string name;
        string workflow_json;
        string trigger_type;
        string trigger_config;
        string sandbox_config_json;
        uint8 target_kind;
        string target_sandbox_id;
        uint64 target_service_id;
    }

    struct WorkflowControlRequest {
        uint64 workflow_id;
    }

    struct SandboxIdRequest {
        string sandbox_id;
    }

    struct SandboxCreateOutput {
        string sandboxId;
        string json;
    }

    struct WorkflowConfig {
        string name;
        string workflow_json;
        string trigger_type;
        string trigger_config;
        string sandbox_config_json;
        uint8 target_kind;
        string target_sandbox_id;
        uint64 target_service_id;
        bool active;
        uint64 created_at;
        uint64 updated_at;
        uint64 last_triggered_at;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // EVENTS (emitted from libraries when they mutate state; the blueprint
    // re-declares the same signatures so the ABI surface is unchanged)
    // ═══════════════════════════════════════════════════════════════════════════

    event SandboxCreated(bytes32 indexed sandboxHash, address indexed operator);
    event SandboxDeleted(bytes32 indexed sandboxHash, address indexed operator);
    event WorkflowStored(uint64 indexed workflow_id, string trigger_type, string trigger_config);
    event WorkflowTriggered(uint64 indexed workflow_id, uint64 triggered_at);
    event WorkflowCanceled(uint64 indexed workflow_id, uint64 canceled_at);
    event OperatorProvisioned(uint64 indexed serviceId, address indexed operator, string sandboxId, string sidecarUrl);
    event OperatorDeprovisioned(uint64 indexed serviceId, address indexed operator);
    event TeeAttestationStored(uint64 indexed serviceId, address indexed operator, bytes32 attestationHash);

    // ═══════════════════════════════════════════════════════════════════════════
    // ERRORS
    // ═══════════════════════════════════════════════════════════════════════════

    error NoAvailableCapacity();
    error OperatorMismatch(address expected, address actual);
    error SandboxNotFound(bytes32 sandboxHash);
    error SandboxAlreadyExists(bytes32 sandboxHash);
    error EmptySandboxId();
    error SandboxIdTooLong(uint256 length);
    error WorkflowNotFound(uint64 workflowId);
    error MaxWorkflowsReached(uint64 serviceId);
    error InvalidWorkflowTarget(uint8 targetKind);

    error AlreadyProvisioned(uint64 serviceId, address operator);
    error NotProvisioned(uint64 serviceId, address operator);
    error MissingTeeAttestation(uint64 serviceId, address operator);
    error MaxOperatorsReached(uint64 serviceId);

    error RestakingNotSet();
}
