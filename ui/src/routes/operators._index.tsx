import { useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { Link } from 'react-router';
import type { DiscoveredOperator } from '@tangle-network/blueprint-ui';
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
import {
  INSTANCE_ONCHAIN_BLUEPRINT_ID,
  SANDBOX_ONCHAIN_BLUEPRINT_ID,
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
} from '~/lib/config';
import { useReliableOperators, type ReliableOperatorsResult } from '~/lib/hooks/useReliableOperators';

type OperatorRow = {
  operator: string;
  blueprintId: string;
  blueprintLabel: string;
  blueprintParam: string;
  rpcAddress: string;
  registered: boolean;
  resources: number;
  running: number;
  tee: number;
  backends: string;
  lastSeen: number;
};

type LocalOperatorStats = Pick<OperatorRow, 'resources' | 'running' | 'tee' | 'backends' | 'lastSeen'>;

function zeroStats(): LocalOperatorStats {
  return {
    resources: 0,
    running: 0,
    tee: 0,
    backends: 'docker',
    lastSeen: 0,
  };
}

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
  const sandboxOperators = useReliableOperators(SANDBOX_ONCHAIN_BLUEPRINT_ID || '0');
  const instanceOperators = useReliableOperators(INSTANCE_ONCHAIN_BLUEPRINT_ID || '0');
  const teeOperators = useReliableOperators(TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID || '0');

  const localStats = useMemo<Map<string, LocalOperatorStats>>(() => {
    const byOperator = new Map<string, LocalOperatorStats>();
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
      const current = byOperator.get(operator) ?? zeroStats();
      current.resources += 1;
      current.running += resource.status === 'running' ? 1 : 0;
      current.tee += resource.tee ? 1 : 0;
      current.backends = resource.tee ? 'docker · tee' : current.backends;
      current.lastSeen = Math.max(current.lastSeen, resource.timestamp);
      byOperator.set(operator, current);
    }

    return byOperator;
  }, [instances, sandboxes]);

  const rows = useMemo<OperatorRow[]>(() => {
    const groups: Array<{
      blueprintId: string;
      blueprintLabel: string;
      blueprintParam: string;
      query: ReliableOperatorsResult;
    }> = [
      {
        blueprintId: SANDBOX_ONCHAIN_BLUEPRINT_ID,
        blueprintLabel: 'Sandbox',
        blueprintParam: 'ai-agent-sandbox-blueprint',
        query: sandboxOperators,
      },
      {
        blueprintId: INSTANCE_ONCHAIN_BLUEPRINT_ID,
        blueprintLabel: 'Instance',
        blueprintParam: 'ai-agent-instance-blueprint',
        query: instanceOperators,
      },
      {
        blueprintId: TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
        blueprintLabel: 'TEE Instance',
        blueprintParam: 'ai-agent-tee-instance-blueprint',
        query: teeOperators,
      },
    ];

    return groups
      .flatMap((group) => group.query.operators.map((operator: DiscoveredOperator) => {
        const stats = localStats.get(operator.address) ?? zeroStats();
        return {
          operator: operator.address,
          blueprintId: group.blueprintId,
          blueprintLabel: group.blueprintLabel,
          blueprintParam: group.blueprintParam,
          rpcAddress: operator.rpcAddress,
          registered: true,
          ...stats,
        };
      }))
      .sort((left, right) => {
        if (right.resources !== left.resources) return right.resources - left.resources;
        return left.blueprintLabel.localeCompare(right.blueprintLabel);
      });
  }, [instanceOperators, localStats, sandboxOperators, teeOperators]);

  const groups = [sandboxOperators, instanceOperators, teeOperators];
  const loading = groups.some((query) => query.isLoading);
  const lookupErrors = groups.filter((query) => query.listError && query.source !== 'service-membership').length;
  const fallbackReads = groups.filter((query) => query.source === 'service-membership').length;
  const registeredCount = sandboxOperators.operatorCount + instanceOperators.operatorCount + teeOperators.operatorCount;
  const uniqueOperatorCount = new Set(rows.map((row) => row.operator.toLowerCase())).size;
  const metrics: ConsoleMetric[] = [
    { label: 'Sandbox slots', value: capacity == null ? '--' : String(capacity), detail: 'BSM capacity', tone: 'brand' },
    { label: 'Blueprint registrations', value: loading && registeredCount === 0n ? '--' : registeredCount.toString(), detail: 'on-chain counts', tone: registeredCount > 0n ? 'ready' : 'warn' },
    { label: 'Operator accounts', value: String(uniqueOperatorCount), detail: fallbackReads > 0 ? 'service-verified' : 'event index', tone: uniqueOperatorCount > 0 ? 'ready' : 'warn' },
    { label: 'Directory status', value: lookupErrors > 0 ? 'blocked' : fallbackReads > 0 ? 'fallback' : 'live', detail: fallbackReads > 0 ? `${fallbackReads} log scans` : 'operator reads', tone: lookupErrors > 0 ? 'danger' : fallbackReads > 0 ? 'warn' : 'ready' },
  ];

  return (
    <ConsolePage title="Operators" eyebrow="Blueprint services">
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={metrics} />
        <ConsoleSection title="Registered Operators">
          {rows.length > 0 ? (
            <div className="overflow-auto">
              <table className="min-w-[920px] w-full border-collapse">
                <thead>
                  <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
                    {['Blueprint', 'Operator', 'RPC', 'Resources', 'Running', 'TEE', 'Backends', 'Last seen', 'Service'].map((label) => (
                      <th key={label} className="px-3 py-2 text-left font-data text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
                        {label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={`${row.blueprintId}:${row.operator}`} className="border-b border-[var(--sandbox-console-border)] hover:bg-[var(--sandbox-console-surface)]">
                      <td className="px-3 py-3">
                        <div className="space-y-1">
                          <p className="font-display text-xs font-semibold text-[var(--sandbox-console-text)]">{row.blueprintLabel}</p>
                          <p className="font-data text-[10px] text-[var(--sandbox-console-subtle)]">#{row.blueprintId}</p>
                        </div>
                      </td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-text)]">{shorten(row.operator)}</td>
                      <td className="max-w-[240px] truncate px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.rpcAddress || 'not advertised'}</td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.resources}</td>
                      <td className="px-3 py-3"><ConsoleChip tone="ready">{row.running}</ConsoleChip></td>
                      <td className="px-3 py-3"><ConsoleChip tone={row.tee > 0 ? 'brand' : 'muted'}>{row.tee}</ConsoleChip></td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.backends}</td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.lastSeen > 0 ? formatAge(row.lastSeen) : '--'}</td>
                      <td className="px-3 py-3">
                        <Link
                          to={`/create?blueprint=${row.blueprintParam}&serviceMode=new`}
                          className="inline-flex h-8 items-center justify-center rounded-[4px] border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] px-3 font-display text-xs font-semibold text-[var(--sandbox-console-text)] transition-colors hover:border-[var(--sandbox-console-brand)] hover:bg-[rgba(142,89,255,0.22)]"
                        >
                          Select
                        </Link>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <EmptyConsoleState
              icon="i-ph:users-three"
              title={loading ? 'Discovering operators' : 'No registered operators found'}
              detail={loading ? 'Reading blueprint registrations from the service manager.' : 'Operator registration happens from the operator runtime; this console will show them once they are registered on-chain.'}
            />
          )}
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
