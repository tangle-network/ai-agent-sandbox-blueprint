import { useMemo, useState } from 'react';
import { useStore } from '@nanostores/react';
import { Link } from 'react-router';
import { cn } from '@tangle-network/blueprint-ui';
import { Button } from '@tangle-network/blueprint-ui/components';
import type { DiscoveredOperator } from '@tangle-network/blueprint-ui';
import {
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
import {
  OperatorIdentity,
} from '~/components/shared/VisualIdentity';

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

type BlueprintFilter = 'all' | OperatorRow['blueprintParam'];
type SortKey = 'blueprint' | 'operator' | 'resources' | 'running' | 'tee' | 'lastSeen';
type SortDirection = 'asc' | 'desc';

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

function formatAge(timestamp: number) {
  const deltaMs = Date.now() - timestamp;
  if (deltaMs < 60_000) return '<1m';
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function compareRows(left: OperatorRow, right: OperatorRow, key: SortKey) {
  if (key === 'operator') return left.operator.localeCompare(right.operator);
  if (key === 'resources') return left.resources - right.resources;
  if (key === 'running') return left.running - right.running;
  if (key === 'tee') return left.tee - right.tee;
  if (key === 'lastSeen') return left.lastSeen - right.lastSeen;
  return left.blueprintLabel.localeCompare(right.blueprintLabel);
}

function SortableHeader({
  label,
  sortKey,
  activeKey,
  direction,
  onSort,
}: {
  label: string;
  sortKey: SortKey;
  activeKey: SortKey;
  direction: SortDirection;
  onSort: (key: SortKey) => void;
}) {
  const active = activeKey === sortKey;

  return (
    <th className="px-3 py-2 text-left">
      <button
        type="button"
        onClick={() => onSort(sortKey)}
        className={cn(
          'inline-flex items-center gap-1.5 font-data text-[11px] font-semibold uppercase tracking-[0.1em] transition-colors',
          active ? 'text-[var(--sandbox-console-text)]' : 'text-[var(--sandbox-console-muted)] hover:text-[var(--sandbox-console-text)]',
        )}
      >
        {label}
        <span className={cn(
          'text-xs transition-transform',
          active ? 'text-[var(--sandbox-console-brand)]' : 'text-[var(--sandbox-console-subtle)]',
          active && direction === 'asc' && 'rotate-180',
          active ? 'i-ph:caret-down-bold' : 'i-ph:caret-up-down',
        )}
        />
      </button>
    </th>
  );
}

export default function OperatorCapacity() {
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const [blueprintFilter, setBlueprintFilter] = useState<BlueprintFilter>('all');
  const [sortKey, setSortKey] = useState<SortKey>('resources');
  const [sortDirection, setSortDirection] = useState<SortDirection>('desc');
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
  const registeredCount = sandboxOperators.operatorCount + instanceOperators.operatorCount + teeOperators.operatorCount;
  const uniqueOperatorCount = new Set(rows.map((row) => row.operator.toLowerCase())).size;
  const runningResources = rows.reduce((total, row) => total + row.running, 0);
  const filterOptions = [
    { value: 'all' as const, label: 'All', count: rows.length },
    { value: 'ai-agent-sandbox-blueprint' as const, label: 'Sandbox', count: rows.filter((row) => row.blueprintParam === 'ai-agent-sandbox-blueprint').length },
    { value: 'ai-agent-instance-blueprint' as const, label: 'Instance', count: rows.filter((row) => row.blueprintParam === 'ai-agent-instance-blueprint').length },
    { value: 'ai-agent-tee-instance-blueprint' as const, label: 'TEE', count: rows.filter((row) => row.blueprintParam === 'ai-agent-tee-instance-blueprint').length },
  ];
  const visibleRows = useMemo(
    () => rows
      .filter((row) => blueprintFilter === 'all' || row.blueprintParam === blueprintFilter)
      .sort((left, right) => {
        const result = compareRows(left, right, sortKey);
        if (result !== 0) return sortDirection === 'asc' ? result : -result;
        return left.operator.localeCompare(right.operator);
      }),
    [blueprintFilter, rows, sortDirection, sortKey],
  );
  const metrics: ConsoleMetric[] = [
    { label: 'Available slots', value: capacity == null ? '--' : String(capacity), tone: 'brand' },
    { label: 'Operators', value: String(uniqueOperatorCount), tone: uniqueOperatorCount > 0 ? 'ready' : 'warn' },
    { label: 'Registrations', value: loading && registeredCount === 0n ? '--' : registeredCount.toString(), tone: registeredCount > 0n ? 'ready' : 'warn' },
    { label: 'Running', value: String(runningResources), tone: runningResources > 0 ? 'ready' : lookupErrors > 0 ? 'danger' : 'muted' },
  ];

  function toggleSort(nextKey: SortKey) {
    if (sortKey === nextKey) {
      setSortDirection((current) => (current === 'asc' ? 'desc' : 'asc'));
      return;
    }
    setSortKey(nextKey);
    setSortDirection(nextKey === 'blueprint' || nextKey === 'operator' ? 'asc' : 'desc');
  }

  return (
    <ConsolePage
      title="Operators"
      eyebrow="Directory"
      actions={(
        <Link to="/operators/register">
          <Button>
            <span className="i-ph:hard-drives text-base" />
            Become an operator
          </Button>
        </Link>
      )}
    >
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={metrics} />
        <ConsoleSection title="Operator Directory">
          {rows.length > 0 ? (
            <>
              <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--sandbox-console-border)] px-3.5 py-3">
                <div className="flex flex-wrap gap-1.5">
                  {filterOptions.map((option) => {
                    const selected = option.value === blueprintFilter;
                    return (
                      <button
                        key={option.value}
                        type="button"
                        aria-label={`${option.label} ${option.count}`}
                        onClick={() => setBlueprintFilter(option.value)}
                        className={cn(
                          'inline-flex h-8 items-center gap-2 rounded-[4px] border px-2.5 font-display text-sm font-bold transition-[background-color,border-color,box-shadow,color]',
                          selected
                            ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                            : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)]',
                        )}
                      >
                        <span>{option.label}</span>
                        <span className="font-data text-xs text-[var(--sandbox-console-muted)]">{option.count}</span>
                      </button>
                    );
                  })}
                </div>
                <p className="font-data text-sm font-semibold text-[var(--sandbox-console-muted)]">
                  {visibleRows.length} shown
                </p>
              </div>
              <div className="overflow-auto">
              <table className="min-w-[980px] w-full border-collapse">
                <thead>
                  <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
                    <SortableHeader label="Blueprint" sortKey="blueprint" activeKey={sortKey} direction={sortDirection} onSort={toggleSort} />
                    <SortableHeader label="Operator" sortKey="operator" activeKey={sortKey} direction={sortDirection} onSort={toggleSort} />
                    <th className="px-3 py-2 text-left font-data text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--sandbox-console-muted)]">RPC</th>
                    <SortableHeader label="Resources" sortKey="resources" activeKey={sortKey} direction={sortDirection} onSort={toggleSort} />
                    <SortableHeader label="Running" sortKey="running" activeKey={sortKey} direction={sortDirection} onSort={toggleSort} />
                    <SortableHeader label="TEE" sortKey="tee" activeKey={sortKey} direction={sortDirection} onSort={toggleSort} />
                    <th className="px-3 py-2 text-left font-data text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--sandbox-console-muted)]">Runtime</th>
                    <SortableHeader label="Last seen" sortKey="lastSeen" activeKey={sortKey} direction={sortDirection} onSort={toggleSort} />
                    <th className="px-3 py-2 text-left font-data text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--sandbox-console-muted)]">Launch</th>
                  </tr>
                </thead>
                <tbody>
                  {visibleRows.map((row) => (
                    <tr key={`${row.blueprintId}:${row.operator}`} className="border-b border-[var(--sandbox-console-border)] hover:bg-[var(--sandbox-console-surface)]">
                      <td className="px-3 py-3">
                        <div className="min-w-0">
                          <p className="truncate font-display text-base font-bold text-[var(--sandbox-console-text)]">{row.blueprintLabel}</p>
                          <p className="font-data text-xs text-[var(--sandbox-console-subtle)]">#{row.blueprintId}</p>
                        </div>
                      </td>
                      <td className="px-3 py-3">
                        <OperatorIdentity address={row.operator} compact />
                      </td>
                      <td className="max-w-[260px] truncate px-3 py-3 font-data text-sm text-[var(--sandbox-console-secondary)]">{row.rpcAddress || 'not advertised'}</td>
                      <td className="px-3 py-3 font-data text-base font-bold text-[var(--sandbox-console-text)]">{row.resources}</td>
                      <td className="px-3 py-3 font-data text-base font-bold text-[var(--sandbox-console-text)]">{row.running}</td>
                      <td className="px-3 py-3 font-data text-base font-bold text-[var(--sandbox-console-text)]">{row.tee}</td>
                      <td className="px-3 py-3 font-data text-sm text-[var(--sandbox-console-secondary)]">{row.backends}</td>
                      <td className="px-3 py-3 font-data text-sm text-[var(--sandbox-console-secondary)]">{row.lastSeen > 0 ? formatAge(row.lastSeen) : '--'}</td>
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
            </>
          ) : (
            <EmptyConsoleState
              icon="i-ph:users-three"
              title={loading ? 'Discovering operators' : 'No registered operators yet'}
              detail={loading ? 'Reading blueprint registrations from the service manager.' : 'Operators appear here once they register on-chain and advertise capacity. Run a node to be the first.'}
              actionTo={loading ? undefined : '/operators/register'}
              actionLabel={loading ? undefined : 'Become an operator'}
            />
          )}
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
