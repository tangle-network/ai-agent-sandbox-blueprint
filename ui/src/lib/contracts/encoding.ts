import { decodeAbiParameters } from 'viem';

/**
 * ABI encoding/decoding helpers for Tangle blueprint jobs.
 *
 * For ENCODING, use the generic encoder:
 *   import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
 */

// Re-export the generic encoder for convenience
export { encodeJobArgs } from './generic-encoder';

// ── Result Decoding ──

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
