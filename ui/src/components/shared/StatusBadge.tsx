import { Badge } from '~/components/ui/badge';
import type { SandboxStatus } from '~/lib/types/sandbox';

const statusConfig: Record<SandboxStatus, { label: string; variant: 'running' | 'stopped' | 'cold' | 'destructive' | 'secondary'; dot: string }> = {
  running: { label: 'Running', variant: 'running', dot: 'status-running' },
  stopped: { label: 'Stopped', variant: 'stopped', dot: 'status-stopped' },
  warm: { label: 'Warm', variant: 'stopped', dot: 'status-warm' },
  cold: { label: 'Cold', variant: 'cold', dot: 'status-cold' },
  gone: { label: 'Deleted', variant: 'secondary', dot: 'status-deleted' },
  error: { label: 'Error', variant: 'destructive', dot: 'status-error' },
};

export function StatusBadge({ status }: { status: SandboxStatus }) {
  const config = statusConfig[status];
  return (
    <Badge variant={config.variant}>
      <span className={`status-dot ${config.dot}`} />
      {config.label}
    </Badge>
  );
}
