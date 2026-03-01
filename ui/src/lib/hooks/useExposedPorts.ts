import { useEffect, useState } from 'react';

interface ExposedPort {
  container_port: number;
  host_port: number;
  protocol: string;
}

type OperatorApiCall = (
  action: string,
  body?: Record<string, unknown>,
  opts?: { method?: string },
) => Promise<Response>;

export function useExposedPorts(status: string | undefined, operatorApiCall: OperatorApiCall) {
  const [ports, setPorts] = useState<ExposedPort[] | null>(null);

  useEffect(() => {
    if (status !== 'running' && status !== 'creating') return;

    let cancelled = false;
    operatorApiCall('ports', undefined, { method: 'GET' })
      .then((res) => res.json())
      .then((data) => {
        if (!cancelled && Array.isArray(data)) setPorts(data);
      })
      .catch(() => { /* ports endpoint may not exist — ignore */ });

    return () => { cancelled = true; };
  }, [status, operatorApiCall]);

  return ports;
}
