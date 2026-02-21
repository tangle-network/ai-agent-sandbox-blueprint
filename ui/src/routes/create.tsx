import { useState, useCallback, useMemo, useEffect } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Badge } from '~/components/ui/badge';
import { InfrastructureModal, InfraBar } from '~/components/shared/InfrastructureModal';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { infraStore, updateInfra } from '~/lib/stores/infra';
import { BlueprintJobForm, type FormSection } from '~/components/forms/BlueprintJobForm';

import { useJobForm } from '~/lib/hooks/useJobForm';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { useJobPrice } from '~/lib/hooks/useJobPrice';
import { useServiceValidation } from '~/lib/hooks/useServiceValidation';
import { formatCost } from '~/lib/hooks/useQuotes';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getAllBlueprints, getBlueprint, type BlueprintDefinition, type JobDefinition } from '~/lib/blueprints';
import { addSandbox, updateSandboxStatus } from '~/lib/stores/sandboxes';
import { addInstance, updateInstanceStatus } from '~/lib/stores/instances';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { cn } from '~/lib/utils';

// ── Blueprint → on-chain ID mapping from env vars ──

const BLUEPRINT_INFRA: Record<string, { blueprintId: string; serviceId: string }> = {
  'ai-agent-sandbox-blueprint': {
    blueprintId: import.meta.env.VITE_SANDBOX_BLUEPRINT_ID ?? '1',
    serviceId: import.meta.env.VITE_SANDBOX_SERVICE_ID ?? '1',
  },
  'ai-agent-instance-blueprint': {
    blueprintId: import.meta.env.VITE_INSTANCE_BLUEPRINT_ID ?? '2',
    serviceId: import.meta.env.VITE_INSTANCE_SERVICE_ID ?? '2',
  },
  'ai-agent-tee-instance-blueprint': {
    blueprintId: import.meta.env.VITE_TEE_INSTANCE_BLUEPRINT_ID ?? '3',
    serviceId: import.meta.env.VITE_INSTANCE_SERVICE_ID ?? '2',
  },
};

// ── Form sections for provision/create jobs (organized layout) ──

const PROVISION_SECTIONS: FormSection[] = [
  { label: 'Identity', fields: ['name', 'agentIdentifier'] },
  { label: 'Image & Stack', fields: ['image', 'stack'] },
  { label: 'Resources', fields: ['cpuCores', 'memoryMb', 'diskGb'] },
  { label: 'Timeouts', fields: ['maxLifetimeSeconds', 'idleTimeoutSeconds'] },
  { label: 'Features', fields: ['sshEnabled', 'sshPublicKey', 'webTerminalEnabled'] },
  { label: 'Advanced Options', fields: ['envJson', 'metadataJson', 'teeRequired', 'teeType'], collapsed: true },
];

// ── Wizard Steps ──

type WizardStep = 'blueprint' | 'configure' | 'deploy';

const STEPS: { key: WizardStep; label: string; icon: string }[] = [
  { key: 'blueprint', label: 'Blueprint', icon: 'i-ph:cube' },
  { key: 'configure', label: 'Configure', icon: 'i-ph:gear' },
  { key: 'deploy', label: 'Deploy', icon: 'i-ph:lightning' },
];

export default function CreatePage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { address } = useAccount();
  const infra = useStore(infraStore);
  const { submitJob, status: txStatus, error: txError, txHash, callId, reset: resetTx } = useSubmitJob();
  const { validate: validateService, isValidating: serviceValidating, serviceInfo, error: serviceError } = useServiceValidation();
  const { data: capacity } = useAvailableCapacity();

  // Pre-select from query params
  const preselectedId = searchParams.get('blueprint');
  const preselected = preselectedId ? getBlueprint(preselectedId) : undefined;

  const [selectedBlueprint, setSelectedBlueprint] = useState<BlueprintDefinition | undefined>(preselected);
  const [step, setStep] = useState<WizardStep>(preselected ? 'configure' : 'blueprint');
  const [showInfra, setShowInfra] = useState(false);
  const [provisionCallId, setProvisionCallId] = useState<number | null>(null);

  // Auto-set infra for preselected blueprint (from query param)
  useEffect(() => {
    if (preselected) {
      const mapping = BLUEPRINT_INFRA[preselected.id];
      if (mapping) {
        updateInfra({ blueprintId: mapping.blueprintId, serviceId: mapping.serviceId, serviceValidated: false });
        validateService(BigInt(mapping.serviceId), address);
      }
    }
  }, []);

  const isSandbox = selectedBlueprint?.id === 'ai-agent-sandbox-blueprint';
  const isInstance = selectedBlueprint?.id === 'ai-agent-instance-blueprint';
  const isTeeInstance = selectedBlueprint?.id === 'ai-agent-tee-instance-blueprint';
  const entityLabel = isSandbox ? 'Sandbox' : 'Instance';

  // The create/provision job is always job 0 for all blueprints
  const createJob = useMemo<JobDefinition | null>(
    () => selectedBlueprint?.jobs.find((j) => j.id === 0) ?? null,
    [selectedBlueprint],
  );

  const { values, errors, onChange, validate, reset: resetForm } = useJobForm(createJob);

  // Set provisionCallId once the tx receipt is parsed
  useEffect(() => {
    if (callId != null) {
      setProvisionCallId(callId);
      const name = String(values.name || '');
      if (name) {
        if (isSandbox) {
          updateSandboxStatus(name, 'creating', { callId });
        } else {
          updateInstanceStatus(name, 'creating', { callId });
        }
      }
    }
  }, [callId]);

  // Watch for OperatorProvisioned event (instances only)
  const instanceProvision = useInstanceProvisionWatcher(
    infra.serviceId ? BigInt(infra.serviceId) : null,
    isTeeInstance ? 'tee-instance' : 'instance',
    txStatus === 'confirmed' && !isSandbox,
  );

  // When instance provision event arrives, update the store
  useEffect(() => {
    if (instanceProvision) {
      const name = String(values.name || '');
      if (name) {
        updateInstanceStatus(name, 'running', {
          id: instanceProvision.sandboxId,
          sidecarUrl: instanceProvision.sidecarUrl,
        });
      }
    }
  }, [instanceProvision]);

  const currentIdx = STEPS.findIndex((s) => s.key === step);
  const canDeploy = createJob && values.name && infra.serviceId;

  // Per-job RFQ: fetch price from operator for the provision/create job
  const operatorRpcUrl = infra.serviceInfo?.operators?.[0]?.rpcAddress;
  const blueprintId = BigInt(infra.blueprintId || '0');
  const serviceIdBig = BigInt(infra.serviceId || '0');
  const { quote: provisionQuote, isLoading: priceLoading, formattedPrice: provisionPriceFormatted } = useJobPrice(
    operatorRpcUrl,
    serviceIdBig,
    createJob?.id ?? 0,
    blueprintId,
    step === 'deploy' && !!operatorRpcUrl && serviceIdBig > 0n && !!createJob,
  );

  // Fallback price from multiplier
  const provisionEstimate = BigInt(createJob?.pricingMultiplier ?? 50) * 1_000_000_000_000_000n;
  const provisionValue = provisionQuote?.price ?? provisionEstimate;
  const hasProvisionRfq = !!provisionQuote;

  // ── Handlers ──

  const handleSelectBlueprint = useCallback((bp: BlueprintDefinition) => {
    setSelectedBlueprint(bp);
    resetForm();
    resetTx();
    // Auto-set on-chain blueprint + service IDs from env vars
    const mapping = BLUEPRINT_INFRA[bp.id];
    if (mapping) {
      updateInfra({
        blueprintId: mapping.blueprintId,
        serviceId: mapping.serviceId,
        serviceValidated: false,
      });
      // Auto-validate the service on-chain
      validateService(BigInt(mapping.serviceId), address);
    }
    setStep('configure');
  }, [resetForm, resetTx, address, validateService]);

  const handleDeploy = useCallback(async () => {
    if (!createJob || !validate()) return;

    const args = encodeJobArgs(createJob, values);
    const name = String(values.name || '');
    const hash = await submitJob({
      serviceId: BigInt(infra.serviceId || '0'),
      jobId: createJob.id,
      args,
      label: `${createJob.label}: ${name}`,
      value: provisionValue,
    });

    if (hash) {
      const common = {
        id: name,
        name,
        image: String(values.image || ''),
        cpuCores: Number(values.cpuCores) || 2,
        memoryMb: Number(values.memoryMb) || 2048,
        diskGb: Number(values.diskGb) || 10,
        createdAt: Date.now(),
        blueprintId: infra.blueprintId,
        serviceId: infra.serviceId,
        status: 'creating' as const,
        txHash: hash,
      };
      if (isSandbox) {
        addSandbox(common);
      } else {
        addInstance({
          ...common,
          teeEnabled: selectedBlueprint?.id === 'ai-agent-tee-instance-blueprint',
        });
      }
    }
  }, [createJob, values, infra, submitJob, validate, isSandbox, selectedBlueprint]);

  return (
    <AnimatedPage className="mx-auto max-w-3xl px-4 sm:px-6 py-8">
      <div className="mb-6">
        <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">
          Create {selectedBlueprint ? entityLabel : 'Resource'}
        </h1>
        <p className="text-sm text-cloud-elements-textSecondary mt-1">
          {selectedBlueprint
            ? selectedBlueprint.description
            : 'Select a blueprint to provision a new AI agent resource on Tangle Network'}
        </p>
      </div>

      {/* Infrastructure bar (shown after blueprint selection) */}
      {step !== 'blueprint' && (
        <>
          <InfraBar onOpenModal={() => setShowInfra(true)} />
          <InfrastructureModal open={showInfra} onOpenChange={setShowInfra} />
        </>
      )}

      {/* Step Indicator */}
      <div className="flex items-center gap-2 mb-8">
        {STEPS.map((s, i) => (
          <div key={s.key} className="flex items-center gap-2 flex-1">
            <button
              onClick={() => i <= currentIdx && setStep(s.key)}
              className={cn(
                'flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-display font-medium transition-all w-full',
                s.key === step
                  ? 'bg-violet-500/10 text-violet-700 dark:text-violet-400 border border-violet-500/20'
                  : i < currentIdx
                    ? 'bg-cloud-elements-background-depth-3 text-cloud-elements-textSecondary border border-cloud-elements-borderColor cursor-pointer'
                    : 'bg-cloud-elements-background-depth-2 text-cloud-elements-textTertiary border border-transparent cursor-default',
              )}
            >
              <div className={`${s.icon} text-base`} />
              <span className="hidden sm:inline">{s.label}</span>
              <span className="sm:hidden">{i + 1}</span>
            </button>
            {i < STEPS.length - 1 && <div className="w-4 h-px bg-cloud-elements-dividerColor shrink-0" />}
          </div>
        ))}
      </div>

      {/* Step 1: Blueprint Selection */}
      {step === 'blueprint' && <BlueprintSelector onSelect={handleSelectBlueprint} />}

      {/* Step 2: Configure */}
      {step === 'configure' && createJob && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <div className={`${selectedBlueprint?.icon} text-lg`} />
                {entityLabel} Configuration
              </CardTitle>
              <CardDescription>Configure your {entityLabel.toLowerCase()} resources and settings</CardDescription>
            </CardHeader>
            <CardContent>
              <BlueprintJobForm
                job={createJob}
                values={values}
                onChange={onChange}
                errors={errors}
                sections={PROVISION_SECTIONS}
              />
            </CardContent>
          </Card>
          <div className="flex justify-between">
            <Button variant="secondary" onClick={() => setStep('blueprint')}>Back</Button>
            <Button onClick={() => setStep('deploy')} disabled={!canDeploy}>Continue</Button>
          </div>
        </div>
      )}

      {/* Step 3: Review & Deploy */}
      {step === 'deploy' && createJob && (
        <DeployStep
          blueprint={selectedBlueprint!}
          job={createJob}
          values={values}
          infra={infra}
          entityLabel={entityLabel}
          isSandbox={isSandbox}
          address={address}
          txStatus={txStatus}
          txHash={txHash}
          txError={txError}
          provisionCallId={provisionCallId}
          instanceProvision={instanceProvision}
          capacity={capacity}
          provisionValue={provisionValue}
          provisionEstimate={provisionEstimate}
          provisionPriceFormatted={provisionPriceFormatted}
          hasProvisionRfq={hasProvisionRfq}
          priceLoading={priceLoading}
          serviceInfo={serviceInfo}
          serviceValidating={serviceValidating}
          serviceError={serviceError}
          onBack={() => { setStep('configure'); resetTx(); }}
          onDeploy={handleDeploy}
          onViewList={() => navigate(isSandbox ? '/sandboxes' : '/instances')}
          onOpenInfra={() => setShowInfra(true)}
          onProvisionReady={(sandboxId, sidecarUrl) => {
            const name = String(values.name || '');
            if (isSandbox) {
              updateSandboxStatus(name, 'running', { id: sandboxId, sidecarUrl });
            } else {
              updateInstanceStatus(name, 'running', { id: sandboxId, sidecarUrl });
            }
          }}
        />
      )}
    </AnimatedPage>
  );
}

// ── Blueprint Selector ──

function BlueprintSelector({ onSelect }: { onSelect: (bp: BlueprintDefinition) => void }) {
  const blueprints = getAllBlueprints();

  const colorMap: Record<string, string> = {
    teal: 'border-teal-500/20 hover:border-teal-500/40',
    blue: 'border-blue-500/20 hover:border-blue-500/40',
    violet: 'border-violet-500/20 hover:border-violet-500/40',
  };

  const iconColorMap: Record<string, string> = {
    teal: 'text-teal-400',
    blue: 'text-blue-400',
    violet: 'text-violet-400',
  };

  return (
    <div className="grid gap-4">
      {blueprints.map((bp) => (
        <button
          key={bp.id}
          onClick={() => onSelect(bp)}
          className={cn(
            'glass-card rounded-xl p-6 text-left transition-all border-2 cursor-pointer',
            'hover:bg-cloud-elements-item-backgroundHover',
            colorMap[bp.color] ?? 'border-cloud-elements-borderColor hover:border-cloud-elements-borderColorActive',
          )}
        >
          <div className="flex items-start gap-4">
            <div className={cn('text-3xl', iconColorMap[bp.color] ?? 'text-cloud-elements-textTertiary', bp.icon)} />
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 mb-1">
                <h3 className="text-lg font-display font-semibold text-cloud-elements-textPrimary">{bp.name}</h3>
                <Badge variant="secondary">v{bp.version}</Badge>
              </div>
              <p className="text-sm text-cloud-elements-textSecondary mb-3">{bp.description}</p>
              <div className="flex items-center gap-4 text-xs text-cloud-elements-textTertiary">
                <span className="flex items-center gap-1">
                  <div className="i-ph:briefcase text-sm" />
                  {bp.jobs.length} jobs
                </span>
                <span className="flex items-center gap-1">
                  <div className="i-ph:folder text-sm" />
                  {bp.categories.length} categories
                </span>
                <span className="flex items-center gap-1">
                  <div className="i-ph:tag text-sm" />
                  {bp.jobs[0]?.pricingMultiplier}x&ndash;{Math.max(...bp.jobs.map((j) => j.pricingMultiplier))}x
                </span>
              </div>
            </div>
            <div className="i-ph:arrow-right text-lg text-cloud-elements-textTertiary" />
          </div>
        </button>
      ))}
    </div>
  );
}

// ── Deploy Step ──

interface DeployStepProps {
  blueprint: BlueprintDefinition;
  job: JobDefinition;
  values: Record<string, unknown>;
  infra: { blueprintId: string; serviceId: string };
  entityLabel: string;
  isSandbox: boolean;
  address?: string;
  txStatus: string;
  txHash?: `0x${string}`;
  txError: string | null;
  provisionCallId: number | null;
  instanceProvision: { sandboxId: string; sidecarUrl: string } | null;
  capacity?: number | bigint;
  provisionValue: bigint;
  provisionEstimate: bigint;
  provisionPriceFormatted: string;
  hasProvisionRfq: boolean;
  priceLoading: boolean;
  serviceInfo: { active: boolean; permitted: boolean } | null;
  serviceValidating: boolean;
  serviceError: string | null;
  onBack: () => void;
  onDeploy: () => void;
  onViewList: () => void;
  onOpenInfra: () => void;
  onProvisionReady: (sandboxId: string, sidecarUrl: string) => void;
}

function DeployStep({
  blueprint, job, values, infra, entityLabel, isSandbox, address,
  txStatus, txHash, txError, provisionCallId, instanceProvision,
  capacity, provisionValue, provisionEstimate, provisionPriceFormatted,
  hasProvisionRfq, priceLoading,
  serviceInfo, serviceValidating, serviceError,
  onBack, onDeploy, onViewList, onOpenInfra, onProvisionReady,
}: DeployStepProps) {
  const [showAllJobs, setShowAllJobs] = useState(false);

  const name = String(values.name || '');
  const image = String(values.image || '');
  const cpuCores = Number(values.cpuCores) || 2;
  const memoryMb = Number(values.memoryMb) || 2048;
  const diskGb = Number(values.diskGb) || 10;
  const costDisplay = hasProvisionRfq ? provisionPriceFormatted : `~${formatCost(provisionEstimate)}`;

  // Separate config fields into key settings vs advanced
  const visibleFields = job.fields.filter((f) => !f.internal);
  const keyFieldNames = new Set(['name', 'image', 'stack', 'cpuCores', 'memoryMb', 'diskGb']);
  const extraFields = visibleFields.filter((f) => !keyFieldNames.has(f.name));
  const activeExtras = extraFields.filter((f) => {
    const v = values[f.name];
    if (f.type === 'boolean') return !!v;
    return v != null && v !== '' && v !== '{}' && v !== f.default;
  });

  const otherJobs = blueprint.jobs.filter((j) => j.id !== job.id);

  return (
    <div className="space-y-4">
      {/* ── Header: What you're deploying ── */}
      <div className="glass-card rounded-xl p-5">
        <div className="flex items-start gap-4">
          <div className="w-12 h-12 rounded-lg bg-violet-500/10 flex items-center justify-center shrink-0">
            <div className={cn('text-2xl text-violet-400', blueprint.icon)} />
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 mb-0.5">
              <h3 className="text-lg font-display font-bold text-cloud-elements-textPrimary">{name || entityLabel}</h3>
              <Badge variant="accent">{blueprint.name}</Badge>
            </div>
            <p className="text-xs font-data text-cloud-elements-textTertiary">{image}</p>
          </div>
          <div className="text-right shrink-0">
            <p className="text-lg font-data font-bold text-cloud-elements-textPrimary">{costDisplay}</p>
            <p className="text-[10px] text-cloud-elements-textTertiary uppercase tracking-wider">deploy cost</p>
          </div>
        </div>

        {/* Resource pills */}
        <div className="flex items-center gap-2 mt-4">
          <ResourcePill icon="i-ph:cpu" label={`${cpuCores} CPU`} />
          <ResourcePill icon="i-ph:memory" label={`${memoryMb} MB`} />
          <ResourcePill icon="i-ph:hard-drive" label={`${diskGb} GB`} />
          <div className="ml-auto flex items-center gap-1.5 text-xs">
            {serviceValidating ? (
              <>
                <div className="w-3 h-3 rounded-full border border-cloud-elements-textTertiary border-t-transparent animate-spin" />
                <span className="text-cloud-elements-textTertiary">Checking service...</span>
              </>
            ) : serviceInfo?.active && serviceInfo?.permitted ? (
              <>
                <div className="i-ph:check-circle-fill text-sm text-teal-400" />
                <span className="text-teal-400">Service #{infra.serviceId}</span>
              </>
            ) : serviceInfo && !serviceInfo.active ? (
              <>
                <div className="i-ph:x-circle text-sm text-crimson-400" />
                <span className="text-crimson-400">Service #{infra.serviceId} inactive</span>
              </>
            ) : serviceInfo && !serviceInfo.permitted ? (
              <>
                <div className="i-ph:warning text-sm text-amber-400" />
                <span className="text-amber-400">Not permitted</span>
              </>
            ) : serviceError ? (
              <>
                <div className="i-ph:x-circle text-sm text-crimson-400" />
                <span className="text-crimson-400">Service not found</span>
              </>
            ) : (
              <>
                <div className="i-ph:globe-simple text-sm text-cloud-elements-textTertiary" />
                <span className="text-cloud-elements-textTertiary">Service #{infra.serviceId}</span>
              </>
            )}
          </div>
        </div>

        {/* Active config options (non-default) */}
        {activeExtras.length > 0 && (
          <div className="mt-3 pt-3 border-t border-white/[0.04] flex flex-wrap gap-1.5">
            {activeExtras.map((f) => {
              const v = values[f.name];
              const display = f.type === 'boolean' ? f.label : `${f.label}: ${
                f.type === 'select' && f.options
                  ? (f.options.find((o) => o.value === String(v))?.label ?? String(v))
                  : String(v)
              }`;
              return (
                <span key={f.name} className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-white/[0.04] text-[11px] font-data text-cloud-elements-textSecondary">
                  <div className="i-ph:check text-[10px] text-teal-400" />
                  {display}
                </span>
              );
            })}
          </div>
        )}
      </div>

      {/* ── Per-job pricing (collapsible) ── */}
      <div className="glass-card rounded-xl overflow-hidden">
        <button
          onClick={() => setShowAllJobs(!showAllJobs)}
          className="w-full flex items-center justify-between px-5 py-3 text-left hover:bg-white/[0.02] transition-colors"
        >
          <div className="flex items-center gap-2">
            <div className="i-ph:receipt text-sm text-cloud-elements-textTertiary" />
            <span className="text-xs font-display font-medium text-cloud-elements-textSecondary">
              Per-job pricing ({otherJobs.length} operations)
            </span>
          </div>
          <div className={cn('i-ph:caret-down text-xs text-cloud-elements-textTertiary transition-transform', showAllJobs && 'rotate-180')} />
        </button>
        {showAllJobs && (
          <div className="px-5 pb-3 space-y-1">
            {otherJobs.map((j) => (
              <div key={j.id} className="flex items-center justify-between py-1">
                <span className="text-xs text-cloud-elements-textSecondary truncate mr-2">{j.label}</span>
                <JobPriceBadge jobIndex={j.id} pricingMultiplier={j.pricingMultiplier} compact />
              </div>
            ))}
          </div>
        )}
      </div>

      {/* ── Capacity ── */}
      {capacity !== undefined && (
        <div className="flex items-center gap-2 px-1">
          <div className="i-ph:shield-check text-sm text-teal-400" />
          <span className="text-xs text-cloud-elements-textTertiary">
            <span className="font-data font-semibold text-cloud-elements-textSecondary">{String(capacity)}</span> capacity slots available
          </span>
        </div>
      )}

      {/* ── TX Status ── */}
      {txStatus !== 'idle' && (
        <div className={cn(
          'rounded-xl border p-4',
          txStatus === 'confirmed' ? 'border-teal-500/20 bg-teal-500/[0.03]'
            : txStatus === 'failed' ? 'border-crimson-500/20 bg-crimson-500/[0.03]'
            : 'border-white/[0.06] bg-white/[0.02]',
        )}>
          <div className="flex items-center gap-3">
            {txStatus === 'signing' && <div className="w-5 h-5 rounded-full border-2 border-amber-400 border-t-transparent animate-spin" />}
            {txStatus === 'pending' && <div className="w-5 h-5 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />}
            {txStatus === 'confirmed' && <div className="i-ph:check-circle-fill text-lg text-teal-400" />}
            {txStatus === 'failed' && <div className="i-ph:x-circle-fill text-lg text-crimson-400" />}
            <div className="flex-1 min-w-0">
              <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                {txStatus === 'signing' && 'Confirm in wallet...'}
                {txStatus === 'pending' && 'Confirming on-chain...'}
                {txStatus === 'confirmed' && `${entityLabel} creation confirmed`}
                {txStatus === 'failed' && 'Transaction failed'}
              </p>
              {txHash && (
                <p className="text-[11px] font-data text-cloud-elements-textTertiary mt-0.5 truncate">
                  {txHash}
                </p>
              )}
              {txError && <p className="text-xs text-crimson-400 mt-0.5">{txError}</p>}
            </div>
          </div>
        </div>
      )}

      {/* ── Provision Progress ── */}
      {txStatus === 'confirmed' && isSandbox && provisionCallId && (
        <ProvisionProgress callId={provisionCallId} onReady={onProvisionReady} />
      )}
      {txStatus === 'confirmed' && !isSandbox && (
        <div className={cn(
          'rounded-xl border p-4',
          instanceProvision ? 'border-teal-500/20 bg-teal-500/[0.03]' : 'border-violet-500/20 bg-violet-500/[0.03]',
        )}>
          <div className="flex items-center gap-3">
            {instanceProvision ? (
              <>
                <div className="i-ph:check-circle-fill text-lg text-teal-400" />
                <div>
                  <p className="text-sm font-display font-medium text-teal-400">Instance ready</p>
                  <p className="text-[11px] text-cloud-elements-textTertiary mt-0.5 font-data truncate max-w-sm">
                    {instanceProvision.sidecarUrl}
                  </p>
                </div>
              </>
            ) : (
              <>
                <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />
                <div>
                  <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                    Waiting for operator...
                  </p>
                  <p className="text-[11px] text-cloud-elements-textTertiary mt-0.5">
                    Watching for on-chain provisioning event
                  </p>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {/* ── Service warning ── */}
      {txStatus === 'idle' && (serviceError || (serviceInfo && (!serviceInfo.active || !serviceInfo.permitted))) && (
        <div className="rounded-xl border border-amber-500/20 bg-amber-500/[0.03] p-4">
          <div className="flex items-center gap-3">
            <div className="i-ph:warning-circle text-lg text-amber-400" />
            <div className="flex-1">
              <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                {serviceError
                  ? `Service #${infra.serviceId} not found`
                  : !serviceInfo?.active
                    ? `Service #${infra.serviceId} is inactive`
                    : `You're not a permitted caller on service #${infra.serviceId}`}
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mt-0.5">
                Open Infrastructure Settings to create a new service or verify a different one.
              </p>
            </div>
            <Button variant="secondary" size="sm" onClick={onOpenInfra}>
              Settings
            </Button>
          </div>
        </div>
      )}

      {/* ── Wallet warning ── */}
      {!address && txStatus === 'idle' && (
        <div className="flex items-center gap-2 px-1">
          <div className="i-ph:wallet text-sm text-amber-400" />
          <span className="text-xs text-amber-400/80">Connect wallet to deploy</span>
        </div>
      )}

      {/* ── Actions ── */}
      <div className="flex justify-between pt-1">
        <Button variant="secondary" onClick={onBack}>Back</Button>
        {txStatus === 'confirmed' ? (
          <Button variant="success" onClick={onViewList}>
            <div className="i-ph:check-bold text-sm" />
            View {entityLabel}s
          </Button>
        ) : (
          <Button
            size="lg"
            onClick={onDeploy}
            disabled={!address || txStatus === 'signing' || txStatus === 'pending' || priceLoading || serviceValidating || !serviceInfo?.active || !serviceInfo?.permitted}
          >
            {(txStatus === 'signing' || txStatus === 'pending') ? (
              <>
                <div className="w-4 h-4 rounded-full border-2 border-white/40 border-t-white animate-spin" />
                Deploying...
              </>
            ) : priceLoading ? (
              'Loading price...'
            ) : (
              <>
                <div className="i-ph:lightning text-base" />
                Deploy for {costDisplay}
              </>
            )}
          </Button>
        )}
      </div>
    </div>
  );
}

function ResourcePill({ icon, label }: { icon: string; label: string }) {
  return (
    <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-white/[0.04] border border-white/[0.06]">
      <div className={cn('text-xs text-cloud-elements-textTertiary', icon)} />
      <span className="text-xs font-data font-medium text-cloud-elements-textSecondary">{label}</span>
    </div>
  );
}
