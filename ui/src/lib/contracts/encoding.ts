import { decodeAbiParameters } from 'viem';

/**
 * ABI decoding helpers for Tangle blueprint jobs.
 * For encoding, import { encodeJobArgs } from '@tangle-network/blueprint-ui' directly.
 */

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
