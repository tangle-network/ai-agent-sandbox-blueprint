import { Link } from 'react-router';
import { cn } from '@tangle-network/blueprint-ui';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { EmptyConsoleState } from './ConsolePrimitives';
import {
  IdentityMark,
  OperatorIdentity,
  getAgentIdentity,
  getBlueprintIdentity,
  getImageIdentity,
  getResourceIdentity,
  getRuntimeIdentity,
  getSecurityIdentity,
  type IdentityMeta,
} from '~/components/shared/VisualIdentity';

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

function shorten(value: string | undefined) {
  if (!value) return '--';
  if (value.length <= 14) return value;
  return `${value.slice(0, 6)}...${value.slice(-4)}`;
}

function formatImageLabel(value: string) {
  return value
    .replace(/^ghcr\.io\/tangle-network\//, '')
    .replace(/^ghcr\.io\//, '')
    .replace(/^docker\.io\//, '');
}

function resourceIdentity(scope: ResourceExplorerRow['scope']) {
  if (scope === 'TEE') return getBlueprintIdentity('ai-agent-tee-instance-blueprint');
  if (scope === 'Instance') return getBlueprintIdentity('ai-agent-instance-blueprint');
  return getBlueprintIdentity('ai-agent-sandbox-blueprint');
}

function ExplorerIdentityCell({
  identity,
  value,
  detail,
  truncateValue = true,
}: {
  identity: IdentityMeta;
  value: string;
  detail?: string;
  truncateValue?: boolean;
}) {
  return (
    <span className="flex min-w-0 items-center gap-2.5">
      <IdentityMark identity={identity} size="sm" />
      <span className="min-w-0">
        <span className={cn(
          'block font-data text-xs font-bold text-[var(--sandbox-console-text)]',
          truncateValue ? 'truncate' : 'whitespace-nowrap',
        )}
        >
          {value}
        </span>
        {detail ? (
          <span className="block truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
            {detail}
          </span>
        ) : null}
      </span>
    </span>
  );
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
      <table className="w-full table-fixed border-collapse">
        <colgroup>
          <col className="w-[22%]" />
          <col className="w-[9%]" />
          <col className="w-[16%]" />
          <col className="w-[9%]" />
          <col className="w-[13%]" />
          <col className="w-[9%]" />
          <col className="w-[10%]" />
          <col className="w-[12%]" />
        </colgroup>
        <thead>
          <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
            {['Resource', 'Status', 'Image', 'Runtime', 'Operator', 'Spec', 'Agent', 'Trust'].map((label) => (
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
                  <IdentityMark identity={resourceIdentity(row.scope)} size="md" />
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
                <ExplorerIdentityCell
                  identity={getImageIdentity(row.image)}
                  value={formatImageLabel(row.image)}
                />
              </td>
              <td className="px-3 py-3">
                <ExplorerIdentityCell
                  identity={getRuntimeIdentity(row.backend)}
                  value={row.backend}
                  truncateValue={false}
                />
              </td>
              <td className="px-3 py-3">
                {row.operator ? (
                  <OperatorIdentity address={row.operator} compact />
                ) : (
                  <ExplorerIdentityCell identity={getOperatorPlaceholderIdentity()} value="unassigned" detail="operator pending" />
                )}
              </td>
              <td className="px-3 py-3">
                <ExplorerIdentityCell identity={getResourceIdentity('cpu')} value={row.specs} truncateValue={false} />
              </td>
              <td className="px-3 py-3">
                <ExplorerIdentityCell
                  identity={getAgentIdentity(row.agentIdentifier ?? '')}
                  value={row.sessions}
                  truncateValue={false}
                />
              </td>
              <td className="px-3 py-3">
                <ExplorerIdentityCell identity={getSecurityIdentity(row.security)} value={row.security} truncateValue={false} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function getOperatorPlaceholderIdentity(): IdentityMeta {
  return { label: 'Operator pending', mark: 'OP', detail: 'unassigned', image: 'tangle', tone: 'slate' };
}
