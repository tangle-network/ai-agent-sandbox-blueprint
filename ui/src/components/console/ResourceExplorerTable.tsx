import { Link } from 'react-router';
import { cn } from '@tangle-network/blueprint-ui';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { ConsoleChip, EmptyConsoleState } from './ConsolePrimitives';

export type ResourceExplorerRow = {
  key: string;
  href: string;
  name: string;
  id: string;
  scope: 'Sandbox' | 'Instance' | 'TEE';
  status: string;
  statusLabel?: string;
  backend: string;
  image: string;
  operator?: string;
  specs: string;
  sessions: string;
  workflows: string;
  network: string;
  security: string;
  storage: string;
  createdAt: number;
  lastEvent?: number;
  teeEnabled?: boolean;
  agentIdentifier?: string;
};

function formatAge(timestamp: number | undefined) {
  if (!timestamp) return '--';
  const deltaMs = Date.now() - timestamp;
  if (deltaMs < 60_000) return '<1m';
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function shorten(value: string | undefined) {
  if (!value) return '--';
  if (value.length <= 14) return value;
  return `${value.slice(0, 6)}...${value.slice(-4)}`;
}

export function ResourceExplorerTable({
  rows,
  emptyTitle,
  emptyDetail,
  emptyActionTo,
  emptyActionLabel,
}: {
  rows: ResourceExplorerRow[];
  emptyTitle: string;
  emptyDetail: string;
  emptyActionTo: string;
  emptyActionLabel: string;
}) {
  if (rows.length === 0) {
    return (
      <EmptyConsoleState
        icon="i-ph:hard-drives"
        title={emptyTitle}
        detail={emptyDetail}
        actionTo={emptyActionTo}
        actionLabel={emptyActionLabel}
      />
    );
  }

  return (
    <div className="overflow-auto">
      <table className="min-w-[1120px] w-full border-collapse">
        <thead>
          <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
            {['Resource', 'Status', 'Backend', 'Operator', 'Spec', 'Sessions', 'Workflows', 'Network', 'Security', 'Storage', 'Last'].map((label) => (
              <th
                key={label}
                className="px-3 py-2 text-left font-data text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]"
              >
                {label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.key}
              className="group border-b border-[var(--sandbox-console-border)] transition-colors hover:bg-[var(--sandbox-console-surface)]"
            >
              <td className="px-3 py-3">
                <Link to={row.href} className="flex min-w-0 items-center gap-3">
                  <span
                    className={cn(
                      'flex h-8 w-8 shrink-0 items-center justify-center rounded border',
                      row.teeEnabled
                        ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-brand)]'
                        : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] text-[var(--sandbox-console-muted)]',
                    )}
                  >
                    <span className={cn('text-base', row.teeEnabled ? 'i-ph:shield-check' : 'i-ph:cube')} />
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate font-display text-sm font-semibold text-[var(--sandbox-console-text)]">
                      {row.name}
                    </span>
                    <span className="block truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
                      {row.scope} · {shorten(row.id)}{row.agentIdentifier ? ` · ${row.agentIdentifier}` : ''}
                    </span>
                  </span>
                </Link>
              </td>
              <td className="px-3 py-3">
                <StatusBadge status={row.status} labelOverride={row.statusLabel} />
              </td>
              <td className="px-3 py-3">
                <ConsoleChip tone={row.backend === 'tee' ? 'brand' : row.backend === 'firecracker' ? 'warn' : 'muted'}>
                  {row.backend}
                </ConsoleChip>
              </td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{shorten(row.operator)}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.specs}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.sessions}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.workflows}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.network}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.security}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{row.storage}</td>
              <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{formatAge(row.lastEvent ?? row.createdAt)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
