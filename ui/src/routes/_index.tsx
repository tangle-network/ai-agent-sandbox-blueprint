import { Link } from 'react-router';
import { useStore } from '@nanostores/react';
import { AnimatedPage, StaggerContainer, StaggerItem } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Badge } from '~/components/ui/badge';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { sandboxListStore, runningSandboxes, stoppedSandboxes } from '~/lib/stores/sandboxes';
import { useServiceStats, useAvailableCapacity, useWorkflowIds } from '~/lib/hooks/useSandboxReads';
import { cn } from '~/lib/utils';

export default function Dashboard() {
  const sandboxes = useStore(sandboxListStore);
  const running = useStore(runningSandboxes);
  const stopped = useStore(stoppedSandboxes);
  const { data: stats } = useServiceStats();
  const { data: capacity } = useAvailableCapacity();
  const { data: workflowIds } = useWorkflowIds(false);

  const statCards = [
    { label: 'Running', value: String(running.length), icon: 'i-ph:play', color: 'text-teal-400', glow: 'glow-border-teal' },
    { label: 'Stopped', value: String(stopped.length), icon: 'i-ph:stop', color: 'text-amber-400', glow: '' },
    { label: 'Capacity', value: capacity !== undefined ? String(capacity) : '--', icon: 'i-ph:cpu', color: 'text-blue-400', glow: '' },
    { label: 'Workflows', value: workflowIds ? String(workflowIds.length) : '--', icon: 'i-ph:flow-arrow', color: 'text-violet-400', glow: '' },
  ];

  const recentSandboxes = sandboxes.slice(0, 5);

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Dashboard</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">Manage your AI agent sandboxes</p>
        </div>
        <Link to="/create">
          <Button size="lg">
            <div className="i-ph:plus text-base" />
            Create Sandbox
          </Button>
        </Link>
      </div>

      {/* Stats */}
      <StaggerContainer className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-8">
        {statCards.map((stat) => (
          <StaggerItem key={stat.label}>
            <Card className={stat.glow}>
              <CardContent className="p-6">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-xs font-data uppercase tracking-wider text-cloud-elements-textTertiary">{stat.label}</p>
                    <p className="text-3xl font-display font-bold mt-1">{stat.value}</p>
                  </div>
                  <div className={`${stat.icon} text-2xl ${stat.color}`} />
                </div>
              </CardContent>
            </Card>
          </StaggerItem>
        ))}
      </StaggerContainer>

      {/* Network Stats (from contract) */}
      {stats && (
        <Card className="mb-8">
          <CardContent className="p-6">
            <div className="flex items-center gap-6">
              <div className="flex items-center gap-2">
                <div className="i-ph:globe text-lg text-cloud-elements-textTertiary" />
                <span className="text-sm text-cloud-elements-textSecondary">Network:</span>
              </div>
              <div className="flex items-center gap-4">
                <span className="text-sm">
                  <span className="text-cloud-elements-textTertiary">Total Sandboxes:</span>{' '}
                  <span className="font-data font-semibold">{String(stats[0])}</span>
                </span>
                <span className="text-cloud-elements-dividerColor">|</span>
                <span className="text-sm">
                  <span className="text-cloud-elements-textTertiary">Total Capacity:</span>{' '}
                  <span className="font-data font-semibold">{String(stats[1])}</span>
                </span>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Recent Sandboxes */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>Recent Sandboxes</CardTitle>
          {sandboxes.length > 0 && (
            <Link to="/sandboxes">
              <Button variant="ghost" size="sm">View All</Button>
            </Link>
          )}
        </CardHeader>
        <CardContent>
          {recentSandboxes.length > 0 ? (
            <div className="space-y-2">
              {recentSandboxes.map((sb) => (
                <Link
                  key={sb.id}
                  to={`/sandboxes/${encodeURIComponent(sb.id)}`}
                  className="flex items-center justify-between p-3 rounded-lg hover:bg-cloud-elements-item-backgroundHover transition-colors"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="i-ph:hard-drives text-lg text-cloud-elements-textTertiary" />
                    <div className="min-w-0">
                      <p className="text-sm font-display font-medium text-cloud-elements-textPrimary truncate">{sb.name}</p>
                      <p className="text-xs font-data text-cloud-elements-textTertiary">{sb.image}</p>
                    </div>
                  </div>
                  <div className="flex items-center gap-3">
                    <span className="text-xs font-data text-cloud-elements-textTertiary hidden sm:block">
                      {sb.cpuCores} CPU · {sb.memoryMb}MB
                    </span>
                    <StatusBadge status={sb.status === 'creating' ? 'running' : sb.status} />
                  </div>
                </Link>
              ))}
            </div>
          ) : (
            <div className="py-12 text-center">
              <div className="i-ph:cloud text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-cloud-elements-textSecondary font-display">No sandboxes yet</p>
              <p className="text-sm text-cloud-elements-textTertiary mt-1">Create your first sandbox to get started</p>
              <Link to="/create" className="inline-block mt-4">
                <Button variant="outline" size="sm">Create Sandbox</Button>
              </Link>
            </div>
          )}
        </CardContent>
      </Card>
    </AnimatedPage>
  );
}
