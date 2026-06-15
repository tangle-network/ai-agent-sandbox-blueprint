import { Link } from 'react-router';
import type { ReactNode } from 'react';
import { Button } from '@tangle-network/blueprint-ui/components';
import { ConsoleChip, ConsoleSection } from './ConsolePrimitives';
import { IdentityMark, type IdentityMeta } from '~/components/shared/VisualIdentity';

type Tone = 'brand' | 'ready' | 'warn' | 'danger' | 'muted';

export type WorkspaceRailRow = {
  label: string;
  value: string;
  detail?: string;
  tone?: Tone;
  identity?: IdentityMeta;
  leading?: ReactNode;
};

function WorkspaceRow({ row }: { row: WorkspaceRailRow }) {
  return (
    <div className="grid gap-1 border-b border-[var(--sandbox-console-border)] px-3 py-3 transition-colors last:border-b-0 hover:bg-[var(--sandbox-console-hover)]">
      <div className="flex items-center justify-between gap-3">
        <span className="flex min-w-0 items-center gap-2">
          {row.leading ?? (row.identity ? <IdentityMark identity={row.identity} size="sm" /> : null)}
          <span className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
            {row.label}
          </span>
        </span>
        <ConsoleChip tone={row.tone ?? 'muted'}>{row.value}</ConsoleChip>
      </div>
      {row.detail ? (
        <p className="truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
          {row.detail}
        </p>
      ) : null}
    </div>
  );
}

export function ResourceWorkspaceRail({ rows }: { rows: WorkspaceRailRow[] }) {
  return (
    <ConsoleSection title="Resource Context">
      <div>
        {rows.map((row) => (
          <WorkspaceRow key={row.label} row={row} />
        ))}
      </div>
    </ConsoleSection>
  );
}

export function AutomationWorkspace({
  createHref,
  scope,
  target,
  status,
  hasAgent,
}: {
  createHref?: string;
  scope: string;
  target: string;
  status: string;
  hasAgent: boolean;
}) {
  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_300px]">
      <ConsoleSection title="Workflow Lane">
        <div className="divide-y divide-[var(--sandbox-console-border)]">
          {[
            { label: 'Scope', value: scope },
            { label: 'Target', value: target },
            { label: 'State', value: status },
            { label: 'Agent', value: hasAgent ? 'configured' : 'not configured' },
          ].map((row) => (
            <div key={row.label} className="grid grid-cols-[120px_minmax(0,1fr)] gap-3 px-3 py-3">
              <span className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
                {row.label}
              </span>
              <span className="truncate font-data text-xs text-[var(--sandbox-console-text)]">
                {row.value}
              </span>
            </div>
          ))}
        </div>
      </ConsoleSection>

      <ConsoleSection title="Execution Readiness">
        <div className="space-y-3 p-3">
          <ConsoleChip tone={status === 'running' ? 'ready' : 'warn'}>
            {status === 'running' ? 'runtime ready' : 'runtime gated'}
          </ConsoleChip>
          <ConsoleChip tone={hasAgent ? 'brand' : 'warn'}>
            {hasAgent ? 'agent sessions' : 'compute only'}
          </ConsoleChip>
          {createHref ? (
            <Link to={createHref} className="block">
              <Button variant="secondary" size="sm" className="w-full justify-center">
                <span className="i-ph:flow-arrow text-sm" />
                Create Workflow
              </Button>
            </Link>
          ) : null}
        </div>
      </ConsoleSection>
    </div>
  );
}

export function StorageWorkspace({
  rows,
  onSnapshot,
  snapshotEnabled,
}: {
  rows: WorkspaceRailRow[];
  onSnapshot: () => void;
  snapshotEnabled: boolean;
}) {
  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_300px]">
      <ConsoleSection title="Storage Ledger">
        <div>
          {rows.map((row) => (
            <WorkspaceRow key={row.label} row={row} />
          ))}
        </div>
      </ConsoleSection>

      <ConsoleSection title="Snapshot Surface">
        <div className="space-y-3 p-3">
          <ConsoleChip tone={snapshotEnabled ? 'ready' : 'warn'}>
            {snapshotEnabled ? 'snapshot ready' : 'runtime required'}
          </ConsoleChip>
          <Button
            variant="secondary"
            size="sm"
            className="w-full justify-center"
            onClick={onSnapshot}
            disabled={!snapshotEnabled}
          >
            <span className="i-ph:camera text-sm" />
            Snapshot
          </Button>
        </div>
      </ConsoleSection>
    </div>
  );
}
