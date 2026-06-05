import { useMemo } from 'react';
import { useStore } from '@nanostores/react';
import {
  ConsoleChip,
  ConsoleMetricStrip,
  ConsolePage,
  ConsoleSection,
  EmptyConsoleState,
  type ConsoleMetric,
} from '~/components/console/ConsolePrimitives';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import { instanceListStore } from '~/lib/stores/instances';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';

type OperatorRow = {
  operator: string;
  resources: number;
  running: number;
  tee: number;
  backends: string;
  lastSeen: number;
};

function shorten(value: string) {
  if (value.length <= 14) return value;
  return `${value.slice(0, 6)}...${value.slice(-4)}`;
}

function formatAge(timestamp: number) {
  const deltaMs = Date.now() - timestamp;
  if (deltaMs < 60_000) return '<1m';
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

export default function OperatorCapacity() {
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const { data: capacity } = useAvailableCapacity();

  const rows = useMemo<OperatorRow[]>(() => {
    const byOperator = new Map<string, OperatorRow>();
    const resources = [
      ...sandboxes.map((resource) => ({
        operator: resource.operator,
        status: resource.status,
        tee: !!resource.teeEnabled,
        timestamp: resource.lastActivityAt ?? resource.createdAt,
      })),
      ...instances.map((resource) => ({
        operator: resource.operator,
        status: resource.status,
        tee: !!resource.teeEnabled,
        timestamp: resource.createdAt,
      })),
    ];

    for (const resource of resources) {
      if (!resource.operator) continue;
      const operator = resource.operator;
      const current = byOperator.get(operator) ?? {
        operator,
        resources: 0,
        running: 0,
        tee: 0,
        backends: 'docker',
        lastSeen: 0,
      };
      current.resources += 1;
      current.running += resource.status === 'running' ? 1 : 0;
      current.tee += resource.tee ? 1 : 0;
      current.backends = resource.tee ? 'docker · tee' : current.backends;
      current.lastSeen = Math.max(current.lastSeen, resource.timestamp);
      byOperator.set(operator, current);
    }

    return Array.from(byOperator.values()).sort((left, right) => right.resources - left.resources);
  }, [instances, sandboxes]);

  const metrics: ConsoleMetric[] = [
    { label: 'Available capacity', value: capacity == null ? '--' : String(capacity), detail: 'contract read', tone: 'brand' },
    { label: 'Known operators', value: String(rows.length), detail: 'from resources', tone: 'ready' },
    { label: 'Running resources', value: String(rows.reduce((sum, row) => sum + row.running, 0)), detail: 'assigned', tone: 'ready' },
    { label: 'TEE allocations', value: String(rows.reduce((sum, row) => sum + row.tee, 0)), detail: 'attested', tone: 'brand' },
  ];

  return (
    <ConsolePage title="Capacity Directory" eyebrow="Operator route">
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={metrics} />
        <ConsoleSection title="Operators">
          {rows.length > 0 ? (
            <div className="overflow-auto">
              <table className="min-w-[780px] w-full border-collapse">
                <thead>
                  <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
                    {['Operator', 'Resources', 'Running', 'TEE', 'Backends', 'Last seen'].map((label) => (
                      <th key={label} className="px-3 py-2 text-left font-data text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
                        {label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={row.operator} className="border-b border-[var(--sandbox-console-border)] hover:bg-[var(--sandbox-console-surface)]">
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-text)]">{shorten(row.operator)}</td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.resources}</td>
                      <td className="px-3 py-3"><ConsoleChip tone="ready">{row.running}</ConsoleChip></td>
                      <td className="px-3 py-3"><ConsoleChip tone={row.tee > 0 ? 'brand' : 'muted'}>{row.tee}</ConsoleChip></td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.backends}</td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{formatAge(row.lastSeen)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <EmptyConsoleState
              icon="i-ph:hard-drives"
              title="No operators indexed"
              detail="Operator rows appear once local resources include assigned operators. Contract capacity is still shown above when available."
            />
          )}
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
