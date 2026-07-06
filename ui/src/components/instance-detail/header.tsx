import type { Dispatch, SetStateAction } from 'react';
import { Link } from 'react-router';
import { Button } from '@tangle-network/blueprint-ui/components';
import { ResourceIdentity } from '~/components/shared/ResourceIdentity';
import { ConsoleMetricStrip, type ConsoleMetric } from '~/components/console/ConsolePrimitives';
import { IdentityMark, getBlueprintIdentity } from '~/components/shared/VisualIdentity';
import type { LocalInstance } from '~/lib/stores/instances';
import { getInstanceStatusLabel } from '~/lib/instances/display';

interface InstanceHeaderProps {
  inst: LocalInstance;
  bpId: string;
  hasAgent: boolean;
  setSnapshotOpen: Dispatch<SetStateAction<boolean>>;
  workspaceMetrics: ConsoleMetric[];
}

export function InstanceHeader({
  inst,
  bpId,
  hasAgent,
  setSnapshotOpen,
  workspaceMetrics,
}: InstanceHeaderProps) {
  return (
    <>
      {/* Breadcrumb */}
      <div className="flex items-center gap-2 mb-6 text-sm text-cloud-elements-textTertiary">
        <Link to="/instances" className="hover:text-cloud-elements-textSecondary transition-colors">Instances</Link>
        <span>/</span>
        <span className="text-cloud-elements-textPrimary font-display">{inst.name}</span>
      </div>

      {/* Header */}
      <div className="flex items-start mb-6">
        <div className="flex items-center gap-4">
          <IdentityMark identity={getBlueprintIdentity(bpId)} size="lg" className="h-14 w-14 rounded-[6px]" />
          <ResourceIdentity
            name={inst.name}
            status={inst.status}
            statusLabel={getInstanceStatusLabel(inst)}
            teeEnabled={inst.teeEnabled}
            image={inst.image}
            specs={`${inst.cpuCores} CPU · ${inst.memoryMb}MB · ${inst.diskGb}GB`}
            titleClassName="text-xl"
            teeStyle="pill"
          />
        </div>
        {inst.status === 'running' && (
          <div className="ml-auto flex items-center gap-2">
            <Button variant="secondary" size="sm" onClick={() => setSnapshotOpen(true)}>
              <div className="i-ph:camera text-sm" />
              Snapshot
            </Button>
            {inst.serviceId && (
              <Link to={`/workflows/create?target=${encodeURIComponent(`instance:${inst.id}`)}`}>
                <Button variant="secondary" size="sm" title={!hasAgent ? 'No agent configured — workflow executions will fail' : undefined}>
                  <div className="i-ph:flow-arrow text-sm" />
                  Create Workflow
                </Button>
              </Link>
            )}
          </div>
        )}
      </div>

      {inst.circuitBreakerActive && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Sidecar unreachable — circuit breaker active
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            {inst.circuitBreakerProbing
              ? 'Recovery probe in progress\u2026'
              : `Cooldown active — retrying in ~${inst.circuitBreakerRemainingSecs ?? '?'}s`}
          </p>
        </div>
      )}

      <div className="mb-4">
        <ConsoleMetricStrip metrics={workspaceMetrics} />
      </div>
    </>
  );
}

interface InstanceAlertsProps {
  agentConfigured: boolean;
  hasAgentValidationResult: boolean;
  agentIdentifierValid: boolean;
  configuredAgentIdentifier: string;
  agentAvailableList: string;
}

export function InstanceAlerts({
  agentConfigured,
  hasAgentValidationResult,
  agentIdentifierValid,
  configuredAgentIdentifier,
  agentAvailableList,
}: InstanceAlertsProps) {
  return (
    <>
      {agentConfigured && hasAgentValidationResult && !agentIdentifierValid && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Configured agent not available in this image
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            This instance is configured to use <span className="font-data">{configuredAgentIdentifier}</span>, but the running image only reports {agentAvailableList}.
          </p>
        </div>
      )}
    </>
  );
}
