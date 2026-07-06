import { useEffect, useState } from 'react';
import { useAccount } from 'wagmi';
import { cn, formatCost } from '@tangle-network/blueprint-ui';
import {
  IdentityMark,
  getBlueprintIdentity,
  getRuntimeIdentity,
} from '~/components/shared/VisualIdentity';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { normalizeAgentIdentifier } from '~/lib/agents';
import { updateSandboxStatus } from '~/lib/stores/sandboxes';
import {
  executionMetricToneClass,
  formatCapacityValue,
  type ConsoleTone,
} from './support';
import {
  getDeployBlocker,
  getServiceProblem,
  preflightPanelClass,
  preflightToneClass,
  type DeployStepProps,
} from './deploy-support';
import { ServiceSetupPanel } from './deploy-service';
import {
  DeployButton,
  InstanceProvisionCard,
  OperatorList,
  TxStatusCard,
} from './deploy-status';
import { LaunchActionButton } from './launch-fields';

export function DeploySpecPill({
  icon,
  label,
  value,
}: {
  icon: string;
  label: string;
  value: string;
}) {
  return (
    <div className="min-w-0 rounded-[4px] bg-[var(--sandbox-console-control)] px-3 py-2 shadow-[var(--sandbox-console-control-shadow)]">
      <div className="flex items-center gap-2">
        <span className={cn(icon, 'shrink-0 text-sm text-[var(--sandbox-console-brand)]')} />
        <span className="truncate font-display text-sm font-bold text-[var(--sandbox-console-text)]">{label}</span>
      </div>
      <p className="mt-1 truncate font-data text-xs font-semibold text-[var(--sandbox-console-muted)]">{value}</p>
    </div>
  );
}

export function DeployStep({
  blueprint, job, values, ports, infra, entityLabel, deploy,
  capacity, provisionEstimate, provisionPriceFormatted,
  hasProvisionRfq, priceLoading,
  serviceInfo, serviceValidating, serviceError,
  validateService,
  onBack, onDeploy, onViewDetail, onProvisionReady,
}: DeployStepProps) {
  const { address, isConnected, status: walletStatus } = useAccount();
  const isReconnecting = walletStatus === 'reconnecting';
  const [showAllJobs, setShowAllJobs] = useState(false);
  const [provisionError, setProvisionError] = useState<string | null>(null);

  const name = String(values.name || '');
  const image = String(values.image || '');
  const runtimeBackend = String(values.runtimeBackend || 'docker').toLowerCase();
  const runtimeLabel =
    runtimeBackend === 'firecracker'
      ? 'Firecracker'
      : runtimeBackend === 'tee'
        ? 'TEE'
        : 'Docker';
  const cpuCores = Number(values.cpuCores) || 2;
  const memoryMb = Number(values.memoryMb) || 2048;
  const diskGb = Number(values.diskGb) || 10;
  const costDisplay = hasProvisionRfq ? provisionPriceFormatted : `~${formatCost(provisionEstimate)}`;
  const {
    status,
    txHash,
    error,
    isNewService,
    isInstanceMode,
    operators,
    operatorsLoading,
    operatorsError,
    operatorCount,
    provision,
    callId,
    contractsDeployed,
    sandboxDraftKey,
  } = deploy;
  const isSandbox = !isInstanceMode;
  const isActive = status !== 'idle';
  const isComplete = status === 'confirmed' || status === 'ready';


  useEffect(() => {
    setProvisionError(null);
  }, [callId, status]);

  // Separate config fields into key vs advanced
  const visibleFields = job.fields.filter((f) => !f.internal);
  const keyFieldNames = new Set(['name', 'image', 'stack', 'cpuCores', 'memoryMb', 'diskGb']);
  const extraFields = visibleFields.filter((f) => !keyFieldNames.has(f.name));
  const activeExtras = extraFields.filter((f) => {
    const v = values[f.name];
    if (f.type === 'boolean') return !!v;
    return v != null && v !== '' && v !== '{}' && v !== f.defaultValue;
  });
  const configuredAgentIdentifier = normalizeAgentIdentifier(values.agentIdentifier);
  const activeConfigCount = activeExtras.length + (configuredAgentIdentifier ? 1 : 0);
  const deploymentIntent = isNewService ? `Create service + ${entityLabel}` : `Deploy ${entityLabel}`;
  const serviceNeedsSetup = isSandbox && status === 'idle' && (
    !!serviceError ||
    !!(serviceInfo && (!serviceInfo.active || !serviceInfo.permitted))
  );
  const serviceProblem = getServiceProblem({
    serviceInfo,
    serviceError,
    serviceId: infra.serviceId,
    blueprintId: infra.blueprintId,
    address,
  });
  const deployBlocker = getDeployBlocker({
    status,
    serviceValidating,
    serviceProblem,
    contractsDeployed,
    isConnected: isConnected && !!address,
    isReconnecting,
    isSandbox,
    capacity,
    isNewService,
    operatorsLoading,
    operatorCount: operators.length,
    priceLoading,
  });
  const preflightTone: ConsoleTone = isComplete
    ? 'ready'
    : deployBlocker
      ? deployBlocker.tone
      : isNewService
        ? 'brand'
        : 'ready';
  const preflightIcon = isComplete
    ? 'i-ph:check-circle-fill'
    : deployBlocker
      ? deployBlocker.icon
      : isNewService
        ? 'i-ph:plus-circle'
        : 'i-ph:rocket-launch';
  const preflightTitle = isComplete
    ? `${entityLabel} ready`
    : deployBlocker
      ? deployBlocker.title
      : isNewService
        ? 'Ready to create service'
        : `Ready to deploy to service #${infra.serviceId}`;
  const preflightDetail = deployBlocker
    ? deployBlocker.detail
    : isNewService
      ? `${operators.length} operator${operators.length === 1 ? '' : 's'} selected for a dedicated service request.`
      : 'Service is active, your wallet is permitted, and capacity is available.';
  const capacityText = capacity === undefined
    ? 'Capacity unknown'
    : `${formatCapacityValue(capacity)} slot${Number(capacity) === 1 ? '' : 's'} open`;
  const dueNowLabel = hasProvisionRfq ? 'Operator quote' : 'Estimate';

  const otherJobs = blueprint.jobs.filter((j) => j.id !== job.id);

  return (
    <div className="space-y-4">
      <section className={cn(
        'min-w-0 overflow-hidden rounded-[4px] bg-[var(--sandbox-console-surface)] shadow-[0_18px_44px_rgba(0,0,0,0.14)] ring-1',
        preflightTone === 'danger'
          ? 'ring-red-400/30'
          : preflightTone === 'warn'
            ? 'ring-amber-400/30'
            : 'ring-[var(--sandbox-console-border)]',
      )}>
        <div className="grid min-w-0 gap-px bg-[var(--sandbox-console-border)] xl:grid-cols-[minmax(0,1fr)_minmax(300px,380px)]">
          <div className="min-w-0 bg-[var(--sandbox-console-panel)] p-4 sm:p-5">
            <div className="flex items-start gap-4">
              <IdentityMark identity={getBlueprintIdentity(blueprint.id)} size="lg" />
              <div className="min-w-0 flex-1">
                <p className="font-data text-[11px] font-bold uppercase tracking-[0.16em] text-[var(--sandbox-console-muted)]">
                  {deploymentIntent}
                </p>
                <h2 className="mt-1 truncate font-display text-3xl font-black leading-tight tracking-tight text-[var(--sandbox-console-text)] sm:text-4xl">
                  {name || entityLabel}
                </h2>
                <p className="mt-2 truncate font-data text-sm font-semibold text-[var(--sandbox-console-secondary)]">
                  {image}
                </p>
              </div>
            </div>

            <div className="mt-5 grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
              <DeploySpecPill icon="i-ph:cube" label={entityLabel} value={`Blueprint #${infra.blueprintId || '--'}`} />
              <DeploySpecPill icon={getRuntimeIdentity(runtimeBackend).icon ?? 'i-ph:cube'} label={runtimeLabel} value={runtimeBackend === 'tee' ? 'Attested runtime' : 'Standard runtime'} />
              <DeploySpecPill icon="i-ph:cpu" label={`${cpuCores} CPU`} value={`${memoryMb} MB / ${diskGb} GB`} />
              <DeploySpecPill icon="i-ph:plugs-connected" label={ports.length > 0 ? `${ports.length} port${ports.length === 1 ? '' : 's'}` : 'Proxy'} value={ports.length > 0 ? ports.join(', ') : 'Operator managed'} />
            </div>
          </div>

          <div className="flex min-w-0 flex-col justify-between bg-[var(--sandbox-console-panel-strong)] p-4 sm:p-5">
            <div>
              <div className="flex items-start justify-between gap-4">
                <div>
                  <p className="font-data text-[11px] font-bold uppercase tracking-[0.16em] text-[var(--sandbox-console-muted)]">
                    {dueNowLabel}
                  </p>
                  <p className="mt-1 font-data text-4xl font-black leading-none tracking-tight text-[var(--sandbox-console-text)]">
                    {costDisplay}
                  </p>
                </div>
                <div className={cn('rounded-[4px] px-2.5 py-1.5 font-data text-xs font-bold uppercase tracking-[0.08em]', preflightToneClass[preflightTone])}>
                  {isComplete ? 'Ready' : deployBlocker ? 'Blocked' : 'Ready'}
                </div>
              </div>

              <div className={cn('mt-5 rounded-[4px] p-3.5 ring-1', preflightPanelClass[preflightTone])}>
                <div className="flex items-start gap-3">
                  <span className={cn('mt-0.5 text-lg', preflightIcon, executionMetricToneClass[preflightTone])} />
                  <div className="min-w-0">
                    <p className="font-display text-lg font-bold leading-tight text-[var(--sandbox-console-text)]">
                      {preflightTitle}
                    </p>
                    <p className="mt-1 text-sm leading-6 text-[var(--sandbox-console-secondary)]">
                      {preflightDetail}
                    </p>
                    {serviceNeedsSetup && capacity !== undefined && Number(capacity) > 0 ? (
                      <p className="mt-2 font-data text-xs font-semibold text-[var(--sandbox-console-muted)]">
                        {capacityText}; capacity is not the blocker.
                      </p>
                    ) : null}
                  </div>
                </div>
              </div>
            </div>

            <div className="mt-4 flex flex-col gap-2">
              {isComplete ? (
                <LaunchActionButton variant="success" size="lg" className="w-full" onClick={onViewDetail}>
                  <span className="i-ph:check-bold text-sm" />
                  View {entityLabel}
                </LaunchActionButton>
              ) : serviceNeedsSetup ? (
                <ServiceSetupPanel
                  blueprintId={infra.blueprintId}
                  currentServiceId={infra.serviceId}
                  operators={operators}
                  operatorsLoading={operatorsLoading}
                  operatorsError={operatorsError}
                  operatorCount={operatorCount}
                  validateService={validateService}
                />
              ) : (
                <DeployButton
                  status={status}
                  canDeploy={deploy.canDeploy}
                  isNewService={isNewService}
                  priceLoading={priceLoading}
                  serviceValidating={serviceValidating}
                  costDisplay={costDisplay}
                  blockedTitle={deployBlocker?.title}
                  connectWalletBlocked={deployBlocker?.title === 'Connect wallet'}
                  onDeploy={onDeploy}
                />
              )}
              <LaunchActionButton variant="secondary" size="sm" className="w-full" onClick={onBack}>Back to edit</LaunchActionButton>
            </div>
          </div>
        </div>

        {(activeConfigCount > 0 || configuredAgentIdentifier) && (
          <div className="border-t border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] px-4 py-3">
            <div className="flex flex-wrap gap-1.5">
              {activeExtras.map((f) => {
                const v = values[f.name];
                const display = f.type === 'boolean' ? f.label : `${f.label}: ${
                  f.type === 'select' && f.options
                    ? (f.options.find((o) => o.value === String(v))?.label ?? String(v))
                    : String(v)
                }`;
                return (
                  <span key={f.name} className="inline-flex items-center gap-1.5 rounded-[4px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel-strong)] px-2.5 py-1.5 font-data text-xs font-medium text-[var(--sandbox-console-secondary)]">
                    <span className="i-ph:check text-[10px] text-[var(--sandbox-console-success)]" />
                    {display}
                  </span>
                );
              })}
              {configuredAgentIdentifier && (
                <span className="inline-flex items-center gap-1.5 rounded-[4px] border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] px-2.5 py-1.5 font-data text-xs font-semibold text-[var(--sandbox-console-text)]">
                  <span className="i-ph:robot text-[10px] text-[var(--sandbox-console-brand)]" />
                  Agent: {configuredAgentIdentifier}
                </span>
              )}
            </div>
          </div>
        )}
      </section>

      {/* ── Per-job pricing (collapsible) ── */}
      {otherJobs.length > 0 && (
        <div className="sandbox-console-panel overflow-hidden rounded-[5px]">
          <button
            onClick={() => setShowAllJobs(!showAllJobs)}
            className="flex w-full items-center justify-between px-4 py-3 text-left transition-colors hover:bg-[var(--sandbox-console-hover)]"
          >
            <div className="flex items-center gap-2">
              <div className="i-ph:receipt text-base text-[var(--sandbox-console-muted)]" />
              <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">
                Per-job pricing ({otherJobs.length} operations)
              </span>
            </div>
            <div className={cn('i-ph:caret-down text-xs text-[var(--sandbox-console-muted)] transition-transform', showAllJobs && 'rotate-180')} />
          </button>
          {showAllJobs && (
            <div className="border-t border-[var(--sandbox-console-border)] px-4 py-3">
              {otherJobs.map((j) => (
                <div key={j.id} className="flex items-center justify-between gap-3 py-1.5">
                  <span className="truncate font-data text-xs text-[var(--sandbox-console-secondary)]">{j.label}</span>
                  <JobPriceBadge jobIndex={j.id} pricingMultiplier={j.pricingMultiplier} compact />
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {status === 'idle' && runtimeBackend === 'firecracker' && (
        <div className="rounded-[5px] border border-amber-400/25 bg-amber-400/[0.08] p-4">
          <div className="flex items-center gap-3">
            <div className="i-ph:warning-circle text-lg text-amber-400" />
            <div className="flex-1">
              <p className="font-display text-base font-bold text-[var(--sandbox-console-text)]">
                Firecracker requires an operator runtime with Firecracker provisioning enabled
              </p>
              <p className="mt-1 text-sm text-[var(--sandbox-console-muted)]">
                This mode is mutually exclusive with TEE in the current release.
              </p>
            </div>
          </div>
        </div>
      )}

      {/* ── TX Status ── */}
      {isActive && (
        <TxStatusCard
          status={provisionError ? 'failed' : status}
          txHash={txHash}
          error={provisionError ?? error ?? undefined}
          entityLabel={entityLabel}
          isNewService={isNewService}
        />
      )}

      {/* ── Provision Progress ── */}
      {status === 'confirmed' && isSandbox && callId != null && (
        <ProvisionProgress
          callId={callId}
          onReady={onProvisionReady}
          onFailed={(message) => {
            setProvisionError(message);
            if (sandboxDraftKey) updateSandboxStatus(sandboxDraftKey, 'error', { errorMessage: message });
          }}
        />
      )}
      {status === 'confirmed' && isInstanceMode && (
        <InstanceProvisionCard provision={provision} />
      )}

      {/* ── Operators for instance service requests ── */}
      {isNewService && status === 'idle' && (
        <OperatorList
          operators={operators}
          operatorsLoading={operatorsLoading}
          operatorsError={operatorsError}
          operatorCount={operatorCount}
          blueprintId={infra.blueprintId}
          purpose="instance"
        />
      )}

    </div>
  );
}
