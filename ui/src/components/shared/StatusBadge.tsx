import { Badge } from '@tangle-network/blueprint-ui/components';
import type { SandboxStatus } from '~/lib/types/sandbox';

const statusConfig: Record<SandboxStatus, { label: string; variant: 'running' | 'stopped' | 'cold' | 'destructive' | 'secondary' | 'accent'; dot: string }> = {
  creating: { label: 'Creating', variant: 'accent', dot: 'status-creating' },
  running: { label: 'Running', variant: 'running', dot: 'status-running' },
  stopped: { label: 'Stopped', variant: 'stopped', dot: 'status-stopped' },
  warm: { label: 'Warm', variant: 'stopped', dot: 'status-warm' },
  cold: { label: 'Cold', variant: 'cold', dot: 'status-cold' },
  gone: { label: 'Deleted', variant: 'secondary', dot: 'status-deleted' },
  error: { label: 'Error', variant: 'destructive', dot: 'status-error' },
};

export function StatusBadge({ status, labelOverride }: { status: SandboxStatus; labelOverride?: string }) {
  const config = statusConfig[status];
  return (
    <Badge variant={config.variant}>
      <span className={`status-dot ${config.dot}`} />
      {labelOverride ?? config.label}
    </Badge>
  );
}
