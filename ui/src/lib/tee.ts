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

/**
 * Server-evaluated trust verdict for an attestation report.
 *
 * Mirrors `sandbox_runtime::tee::AttestationVerdict`. `verified` is the only
 * trustworthy state: the quote signature chained to a hardware root AND the
 * measurement signed inside the quote matched a pinned expected value AND (if a
 * nonce was supplied) the freshness binding held.
 */
export type AttestationVerdict =
  | { verdict: 'verified' }
  | { verdict: 'unverified'; reason: string }
  | { verdict: 'measurement_mismatch' };

/** Mirrors `sandbox_runtime::tee::AttestationVerification`. */
export interface AttestationVerification {
  verdict: AttestationVerdict;
  signature_verified: boolean;
  measurement_matched: boolean;
  report_data_matched: boolean;
  structural_ok: boolean;
}

/** Response envelope for `GET/POST /api/sandboxes/{id}/tee/attestation`. */
export interface AttestationResponse {
  sandbox_id: string;
  attestation: AttestationData;
  verification: AttestationVerification;
}

/**
 * True only when the attestation carries a `verified` verdict.
 *
 * TRUST BOUNDARY — read before relying on this:
 *
 * This reflects the verdict computed by the operator-run server
 * (`verify_attestation` on the operator's node). It is a HINT, not a
 * client-side cryptographic proof: a malicious operator can return
 * `{verdict:'verified', ...}` for forged evidence, because the quote is not
 * (yet) re-verified on the relying party. The durable fix is to verify the raw
 * `attestation.evidence` against a client-pinned hardware root in the client
 * itself (the `verify.rs` chain shipped as WASM, or a client-controlled verifier
 * service) with a client-supplied nonce — see the P0 finding.
 *
 * Until that lands, this function is the fail-closed gate the UI uses: it
 * returns `true` ONLY for an explicit `verified` verdict, and `false` for null,
 * `unverified`, `measurement_mismatch`, or any malformed value. It MUST NEVER be
 * made to key off operator-supplied raw fields (`signature_verified`,
 * `measurement_matched`, `attestation.measurement`, …); those can be forged
 * independently of the verdict. Callers presenting a green "verified" state from
 * this MUST scope their claim to "verified by the operator's node", not
 * "cryptographically verified by you".
 */
export function isAttestationVerified(v: AttestationVerification | null): boolean {
  return v?.verdict?.verdict === 'verified';
}

/** Convert a byte array to a lowercase hex string. */
export function bytesToHex(bytes: number[]): string {
  return bytes.map((b) => (b & 0xff).toString(16).padStart(2, '0')).join('');
}
