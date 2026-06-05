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
import { pendingWorkflowStore } from '~/lib/stores/pendingWorkflows';

type ActivityEvent = {
  key: string;
  source: string;
  action: string;
  detail: string;
  status: string;
  timestamp: number;
  tone: 'brand' | 'ready' | 'warn' | 'danger' | 'muted';
};

function statusTone(status: string): ActivityEvent['tone'] {
  if (status === 'running') return 'ready';
  if (status === 'creating' || status === 'processing') return 'brand';
  if (status === 'stopped' || status === 'warm' || status === 'cold' || status === 'timed-out') return 'warn';
  if (status === 'error') return 'danger';
  return 'muted';
}

function formatTime(timestamp: number) {
  if (!timestamp) return '--';
  return new Date(timestamp).toLocaleString();
}

export default function ActivityTape() {
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const pendingWorkflows = useStore(pendingWorkflowStore);

  const events = useMemo<ActivityEvent[]>(
    () => [
      ...sandboxes.map((sandbox) => ({
        key: `sandbox:${sandbox.localId}`,
        source: 'sandbox',
        action: sandbox.status,
        detail: sandbox.name,
        status: sandbox.status,
        timestamp: sandbox.lastActivityAt ?? sandbox.createdAt,
        tone: statusTone(sandbox.status),
      })),
      ...instances.map((instance) => ({
        key: `instance:${instance.id}`,
        source: instance.teeEnabled ? 'tee instance' : 'instance',
        action: instance.status,
        detail: instance.name,
        status: instance.status,
        timestamp: instance.createdAt,
        tone: statusTone(instance.status),
      })),
      ...pendingWorkflows.map((workflow) => ({
        key: `workflow:${workflow.key}`,
        source: workflow.scope,
        action: workflow.status,
        detail: workflow.name || `Workflow #${workflow.workflowId}`,
        status: workflow.status,
        timestamp: workflow.createdAt,
        tone: statusTone(workflow.status),
      })),
    ].sort((left, right) => right.timestamp - left.timestamp),
    [instances, pendingWorkflows, sandboxes],
  );

  const metrics: ConsoleMetric[] = [
    { label: 'Events', value: String(events.length), detail: 'local index', tone: 'brand' },
    { label: 'Lifecycle', value: String(events.filter((event) => event.source !== 'workflow').length), detail: 'resources', tone: 'ready' },
    { label: 'Workflow pending', value: String(pendingWorkflows.length), detail: 'operator visibility', tone: 'warn' },
    { label: 'Errors', value: String(events.filter((event) => event.status === 'error').length), detail: 'attention', tone: 'danger' },
  ];

  return (
    <ConsolePage title="Execution Tape" eyebrow="Fleet activity">
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={metrics} />
        <ConsoleSection title="Events">
          {events.length > 0 ? (
            <div className="overflow-auto">
              <table className="min-w-[820px] w-full border-collapse">
                <thead>
                  <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
                    {['Time', 'Source', 'Action', 'Resource', 'Status'].map((label) => (
                      <th key={label} className="px-3 py-2 text-left font-data text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
                        {label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {events.map((event) => (
                    <tr key={event.key} className="border-b border-[var(--sandbox-console-border)] hover:bg-[var(--sandbox-console-surface)]">
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{formatTime(event.timestamp)}</td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">{event.source}</td>
                      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-text)]">{event.action}</td>
                      <td className="px-3 py-3 font-display text-sm font-medium text-[var(--sandbox-console-text)]">{event.detail}</td>
                      <td className="px-3 py-3"><ConsoleChip tone={event.tone}>{event.status}</ConsoleChip></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <EmptyConsoleState
              icon="i-ph:pulse"
              title="No execution events"
              detail="Resource lifecycle and workflow visibility events appear here from the current local index."
            />
          )}
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
