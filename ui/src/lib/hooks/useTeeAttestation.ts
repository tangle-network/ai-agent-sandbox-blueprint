import { useCallback, useState } from 'react';
import type { AttestationData } from '~/lib/tee';

type OperatorApiCall = (
  action: string,
  body?: Record<string, unknown>,
  opts?: { method?: string },
) => Promise<Response>;

export function useTeeAttestation(operatorApiCall: OperatorApiCall) {
  const [attestation, setAttestation] = useState<AttestationData | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchAttestation = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await operatorApiCall('tee/attestation', undefined, { method: 'GET' });
      const data: AttestationData = await res.json();
      setAttestation(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch attestation');
    } finally {
      setBusy(false);
    }
  }, [operatorApiCall]);

  return {
    attestation,
    busy,
    error,
    fetchAttestation,
  };
}
