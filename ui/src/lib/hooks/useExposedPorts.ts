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

function toValidPort(value: unknown): number | null {
  const port = Number(value);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) return null;
  return port;
}

function normalizePortRecord(value: unknown): ExposedPort | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  const containerPort = toValidPort(record.container_port ?? record.containerPort ?? record.container);
  const hostPort = toValidPort(record.host_port ?? record.hostPort ?? record.host);
  if (containerPort == null || hostPort == null) return null;
  const protocol = typeof record.protocol === 'string' && record.protocol.trim()
    ? record.protocol.trim().toLowerCase()
    : 'tcp';
  return {
    container_port: containerPort,
    host_port: hostPort,
    protocol,
  };
}

export function normalizeExposedPorts(data: unknown): ExposedPort[] {
  if (Array.isArray(data)) {
    return data
      .map(normalizePortRecord)
      .filter((port): port is ExposedPort => port != null);
  }

  if (!data || typeof data !== 'object') return [];
  const record = data as Record<string, unknown>;
  const ports = record.ports;

  if (Array.isArray(ports)) {
    return normalizeExposedPorts(ports);
  }

  if (!ports || typeof ports !== 'object') return [];

  return Object.entries(ports as Record<string, unknown>)
    .map(([container, value]) => {
      const containerPort = toValidPort(container);
      if (containerPort == null) return null;

      if (typeof value === 'number' || typeof value === 'string') {
        const hostPort = toValidPort(value);
        return hostPort == null ? null : {
          container_port: containerPort,
          host_port: hostPort,
          protocol: 'tcp',
        };
      }

      const normalized = normalizePortRecord({
        ...(value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}),
        container_port: containerPort,
      });
      return normalized;
    })
    .filter((port): port is ExposedPort => port != null);
}

export function useExposedPorts(status: string | undefined, operatorApiCall: OperatorApiCall) {
  const [ports, setPorts] = useState<ExposedPort[] | null>(null);

  useEffect(() => {
    if (status !== 'running' && status !== 'creating') return;

    let cancelled = false;
    operatorApiCall('ports', undefined, { method: 'GET' })
      .then((res) => res.json())
      .then((data) => {
        if (!cancelled) {
          const normalized = normalizeExposedPorts(data);
          if (normalized.length > 0) setPorts(normalized);
        }
      })
      .catch(() => { /* ports endpoint may not exist — ignore */ });

    return () => { cancelled = true; };
  }, [status, operatorApiCall]);

  return ports;
}
