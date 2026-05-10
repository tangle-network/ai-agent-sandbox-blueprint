import { Badge } from '@tangle-network/blueprint-ui/components';
import type { SandboxStatus } from '~/lib/types/sandbox';

type BadgeVariant = 'running' | 'stopped' | 'cold' | 'destructive' | 'secondary' | 'accent';

const statusConfig: Record<SandboxStatus, { label: string; variant: BadgeVariant; dot: string }> = {
  creating: { label: 'Creating', variant: 'accent', dot: 'status-creating' },
  running: { label: 'Running', variant: 'running', dot: 'status-running' },
  stopped: { label: 'Stopped', variant: 'stopped', dot: 'status-stopped' },
  warm: { label: 'Warm', variant: 'stopped', dot: 'status-warm' },
  cold: { label: 'Cold', variant: 'cold', dot: 'status-cold' },
  gone: { label: 'Deleted', variant: 'secondary', dot: 'status-deleted' },
  error: { label: 'Error', variant: 'destructive', dot: 'status-error' },
};

const fallbackConfig = { label: 'Unknown', variant: 'secondary' as BadgeVariant, dot: 'status-deleted' };

function isKnownStatus(value: string): value is SandboxStatus {
  return value in statusConfig;
}

/// Accept arbitrary strings at the type boundary: the operator API may
/// surface a status string the UI doesn't yet know about (e.g. after an
/// API roll-forward). Render a neutral 'Unknown' badge instead of
/// crashing when that happens.
export function StatusBadge({ status, labelOverride }: { status: string; labelOverride?: string }) {
  const config = isKnownStatus(status) ? statusConfig[status] : fallbackConfig;
  return (
    <Badge variant={config.variant}>
      <span className={`status-dot ${config.dot}`} />
      {labelOverride ?? config.label}
    </Badge>
  );
}
