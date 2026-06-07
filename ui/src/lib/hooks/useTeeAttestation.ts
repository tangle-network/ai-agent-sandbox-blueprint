import { useCallback, useState } from 'react';
import type { AttestationData, AttestationVerification } from '~/lib/tee';

type OperatorApiCall = (
  action: string,
  body?: Record<string, unknown>,
  opts?: { method?: string },
) => Promise<Response>;

/**
 * Parse the `/tee/attestation` response. The server returns
 * `{ sandbox_id, attestation, verification }`; tolerate a bare attestation
 * object (older shape) by treating it as having no server verdict.
 */
function parseAttestationResponse(raw: unknown): {
  attestation: AttestationData;
  verification: AttestationVerification | null;
} {
  if (raw && typeof raw === 'object' && 'attestation' in raw) {
    const env = raw as { attestation: AttestationData; verification?: AttestationVerification };
    return { attestation: env.attestation, verification: env.verification ?? null };
  }
  return { attestation: raw as AttestationData, verification: null };
}

export function useTeeAttestation(operatorApiCall: OperatorApiCall) {
  const [attestation, setAttestation] = useState<AttestationData | null>(null);
  const [verification, setVerification] = useState<AttestationVerification | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchAttestation = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await operatorApiCall('tee/attestation', undefined, { method: 'GET' });
      const { attestation: att, verification: ver } = parseAttestationResponse(await res.json());
      setAttestation(att);
      setVerification(ver);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch attestation');
    } finally {
      setBusy(false);
    }
  }, [operatorApiCall]);

  return {
    attestation,
    verification,
    busy,
    error,
    fetchAttestation,
  };
}
