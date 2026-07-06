import type { Dispatch, SetStateAction } from 'react';
import { Link } from 'react-router';
import { Button } from '@tangle-network/blueprint-ui/components';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { ResourceIdentity } from '~/components/shared/ResourceIdentity';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { ConsoleMetricStrip, type ConsoleMetric } from '~/components/console/ConsolePrimitives';
import { IdentityMark, getBlueprintIdentity } from '~/components/shared/VisualIdentity';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { updateSandboxStatus, type LocalSandbox } from '~/lib/stores/sandboxes';

interface SandboxHeaderProps {
  sb: LocalSandbox;
  hasProvisionedSandbox: boolean;
  isRunning: boolean;
  isCreating: boolean;
  isStopped: boolean;
  isGone: boolean;
  hasAgent: boolean;
  handleStop: () => void;
  handleResume: () => void;
  setSnapshotOpen: Dispatch<SetStateAction<boolean>>;
  handleDelete: () => void;
  workspaceMetrics: ConsoleMetric[];
}

export function SandboxHeader({
  sb,
  hasProvisionedSandbox,
  isRunning,
  isCreating,
  isStopped,
  isGone,
  hasAgent,
  handleStop,
  handleResume,
  setSnapshotOpen,
  handleDelete,
  workspaceMetrics,
}: SandboxHeaderProps) {
  return (
    <>
      {/* Header */}
      <div className="flex items-center gap-2 mb-6 text-sm text-cloud-elements-textTertiary">
        <Link to="/sandboxes" className="hover:text-cloud-elements-textSecondary transition-colors">Sandboxes</Link>
        <span>/</span>
        <span className="text-cloud-elements-textPrimary font-display">{sb.name}</span>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div className="flex items-center gap-4">
          <IdentityMark identity={getBlueprintIdentity(sb.teeEnabled ? 'ai-agent-tee-instance-blueprint' : sb.blueprintId)} size="lg" className="h-14 w-14 rounded-[6px]" />
          <ResourceIdentity
            name={sb.name}
            status={sb.status}
            teeEnabled={sb.teeEnabled}
            image={sb.image}
            specs={`${sb.cpuCores} CPU · ${sb.memoryMb}MB · ${sb.diskGb}GB`}
            titleClassName="text-xl"
            teeStyle="pill"
          />
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          {hasProvisionedSandbox && isRunning && !isCreating && (
            <Button variant="secondary" size="sm" onClick={handleStop}>
              <div className="i-ph:stop text-sm" />
              Stop
            </Button>
          )}
          {hasProvisionedSandbox && isStopped && (
            <Button variant="success" size="sm" onClick={handleResume}>
              <div className="i-ph:play text-sm" />
              Resume
            </Button>
          )}
          {hasProvisionedSandbox && !isGone && (
            <>
              <Button variant="secondary" size="sm" onClick={() => setSnapshotOpen(true)}>
                <div className="i-ph:camera text-sm" />
                Snapshot
              </Button>
              {isRunning && sb.sandboxId && (
                <Link to={`/workflows/create?target=${encodeURIComponent(`sandbox:${sb.sandboxId}`)}`}>
                  <Button variant="secondary" size="sm" title={!hasAgent ? 'No agent configured — workflow executions will fail' : undefined}>
                    <div className="i-ph:flow-arrow text-sm" />
                    Create Workflow
                  </Button>
                </Link>
              )}
              <Button variant="destructive" size="sm" onClick={handleDelete}>
                <div className="i-ph:trash text-sm" />
                Delete
                <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_DELETE} pricingMultiplier={PRICING_TIERS[JOB_IDS.SANDBOX_DELETE]?.multiplier ?? 1} compact />
              </Button>
            </>
          )}
        </div>
      </div>

      <div className="mb-4">
        <ConsoleMetricStrip metrics={workspaceMetrics} />
      </div>
    </>
  );
}

interface SandboxAlertsProps {
  sb: LocalSandbox;
  routeKey: string;
  agentConfigured: boolean;
  hasAgentValidationResult: boolean;
  agentIdentifierValid: boolean;
  configuredAgentIdentifier: string;
  agentAvailableList: string;
}

export function SandboxAlerts({
  sb,
  routeKey,
  agentConfigured,
  hasAgentValidationResult,
  agentIdentifierValid,
  configuredAgentIdentifier,
  agentAvailableList,
}: SandboxAlertsProps) {
  return (
    <>
      {agentConfigured && hasAgentValidationResult && !agentIdentifierValid && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Configured agent not available in this image
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            This sandbox is configured to use <span className="font-data">{configuredAgentIdentifier}</span>, but the running image only reports {agentAvailableList}.
          </p>
        </div>
      )}

      {/* Provision Progress (shown when creating) */}
      {sb.status === 'creating' && sb.callId != null && (
        <ProvisionProgress
          callId={sb.callId}
          className="mb-4"
          onReady={(sandboxId, sidecarUrl) => {
            updateSandboxStatus(routeKey, 'running', { sandboxId, sidecarUrl, errorMessage: undefined });
          }}
          onFailed={(message) => updateSandboxStatus(routeKey, 'error', { errorMessage: message })}
        />
      )}

      {sb.status === 'error' && sb.errorMessage && (
        <div className="mb-4 rounded-xl border border-crimson-500/20 bg-crimson-500/5 p-4">
          <p className="text-sm font-display font-medium text-crimson-300">Provisioning failed</p>
          <p className="mt-1 text-xs text-crimson-200/90">{sb.errorMessage}</p>
        </div>
      )}

      {sb.circuitBreakerActive && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Sidecar unreachable — circuit breaker active
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            {sb.circuitBreakerProbing
              ? 'Recovery probe in progress…'
              : `Cooldown active — retrying in ~${sb.circuitBreakerRemainingSecs ?? '?'}s`}
          </p>
        </div>
      )}
    </>
  );
}
