/**
 * TEE attestation types and helpers.
 *
 * Shared between sandbox and instance detail pages to avoid
 * duplicating the AttestationData type and hex encoding logic.
 */

export interface AttestationData {
  tee_type: string;
  evidence: number[];
  measurement: number[];
  timestamp: number;
}

/** Convert a byte array to a lowercase hex string. */
export function bytesToHex(bytes: number[]): string {
  return bytes.map((b) => (b & 0xff).toString(16).padStart(2, '0')).join('');
}
