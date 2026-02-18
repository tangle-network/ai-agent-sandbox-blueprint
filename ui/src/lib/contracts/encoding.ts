import { decodeAbiParameters } from 'viem';

/**
 * ABI encoding/decoding helpers for Tangle blueprint jobs.
 *
 * For ENCODING, use the generic encoder from './generic-encoder':
 *   import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
 *
 * The per-job encoder functions below are DEPRECATED and only kept
 * for backwards compatibility with pages not yet migrated.
 */

// Re-export the generic encoder for convenience
export { encodeJobArgs } from './generic-encoder';

// ── Result Decoding (still needed) ──

export function decodeJobResult(data: `0x${string}`): { success: boolean; output: string } {
  try {
    const [success, output] = decodeAbiParameters(
      [
        { name: 'success', type: 'bool' },
        { name: 'output', type: 'string' },
      ],
      data,
    );
    return { success, output };
  } catch {
    return { success: false, output: 'Failed to decode result' };
  }
}

export function decodeSandboxCreateResult(data: `0x${string}`): { sandboxId: string; sidecarUrl: string } {
  try {
    const [sandboxId, sidecarUrl] = decodeAbiParameters(
      [
        { name: 'sandboxId', type: 'string' },
        { name: 'sidecarUrl', type: 'string' },
      ],
      data,
    );
    return { sandboxId, sidecarUrl };
  } catch {
    return { sandboxId: '', sidecarUrl: '' };
  }
}

// ── Deprecated Per-Job Encoders ──
// These are kept only for workflows._index.tsx backward compat.
// Workflow ABIs happen to be correct; other job ABIs were stale.

import { encodeAbiParameters } from 'viem';

/** @deprecated Use encodeJobArgs with workflow_create job definition */
export function encodeWorkflowCreate(
  name: string,
  workflowJson: string,
  triggerType: string,
  triggerConfig: string,
  sandboxConfigJson: string,
): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'name', type: 'string' },
      { name: 'workflow_json', type: 'string' },
      { name: 'trigger_type', type: 'string' },
      { name: 'trigger_config', type: 'string' },
      { name: 'sandbox_config_json', type: 'string' },
    ],
    [name, workflowJson, triggerType, triggerConfig, sandboxConfigJson],
  );
}

/** @deprecated Use encodeJobArgs with workflow_trigger/cancel job definition */
export function encodeWorkflowControl(workflowId: bigint): `0x${string}` {
  return encodeAbiParameters([{ name: 'workflow_id', type: 'uint64' }], [workflowId]);
}
