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
import { Identicon } from '~/components/shared/Identicon';

import { useJobForm } from '~/lib/hooks/useJobForm';
import { useJobPrice } from '~/lib/hooks/useJobPrice';
import { useServiceValidation } from '~/lib/hooks/useServiceValidation';
import { formatCost } from '~/lib/hooks/useQuotes';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { useCreateDeploy, type DeployStatus } from '~/lib/hooks/useCreateDeploy';
import { getAllBlueprints, getBlueprint, type BlueprintDefinition, type JobDefinition } from '~/lib/blueprints';
import { updateSandboxStatus } from '~/lib/stores/sandboxes';
import { updateInstanceStatus } from '~/lib/stores/instances';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import type { DiscoveredOperator } from '~/lib/hooks/useOperators';
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
  const { validate: validateService, isValidating: serviceValidating, serviceInfo, error: serviceError } = useServiceValidation();
  const { data: capacity } = useAvailableCapacity();

  // Pre-select from query params
  const preselectedId = searchParams.get('blueprint');
  const preselected = preselectedId ? getBlueprint(preselectedId) : undefined;

  const [selectedBlueprint, setSelectedBlueprint] = useState<BlueprintDefinition | undefined>(preselected);
  const [step, setStep] = useState<WizardStep>(preselected ? 'configure' : 'blueprint');
  const [showInfra, setShowInfra] = useState(false);

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

  // The create/provision job: first lifecycle job that doesn't require an existing resource
  const createJob = useMemo<JobDefinition | null>(() => {
    if (!selectedBlueprint) return null;
    return selectedBlueprint.jobs.find(
      (j) => j.category === 'lifecycle' && !j.requiresSandbox,
    ) ?? null;
  }, [selectedBlueprint]);

  const { values, errors, onChange, validate, reset: resetForm } = useJobForm(createJob);

  // Unified deploy hook — manages both submitJob and requestService paths
  const deploy = useCreateDeploy({ blueprint: selectedBlueprint, job: createJob, values, infra, validate });

  const isSandbox = deploy.mode === 'sandbox';
  const entityLabel = isSandbox ? 'Sandbox' : 'Instance';
  const currentIdx = STEPS.findIndex((s) => s.key === step);

  // Per-job RFQ pricing
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
  const provisionEstimate = BigInt(createJob?.pricingMultiplier ?? 50) * 1_000_000_000_000_000n;
  const hasProvisionRfq = !!provisionQuote;

  // ── Handlers ──

  const handleSelectBlueprint = useCallback((bp: BlueprintDefinition) => {
    setSelectedBlueprint(bp);
    resetForm();
    deploy.reset();
    const mapping = BLUEPRINT_INFRA[bp.id];
    if (mapping) {
      updateInfra({ blueprintId: mapping.blueprintId, serviceId: mapping.serviceId, serviceValidated: false });
      if (mapping.serviceId) {
        validateService(BigInt(mapping.serviceId), address);
      }
    }
    setStep('configure');
  }, [resetForm, deploy.reset, address, validateService]);

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

      {/* Infrastructure bar */}
      {step !== 'blueprint' && (
        <>
          {isSandbox ? (
            <InfraBar onOpenModal={() => setShowInfra(true)} />
          ) : (
            <InstanceInfraBar
              infra={infra}
              operators={deploy.operators}
              operatorsLoading={deploy.operatorsLoading}
              hasValidService={deploy.hasValidService}
              onOpenModal={() => setShowInfra(true)}
            />
          )}
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
            <Button onClick={() => setStep('deploy')} disabled={!createJob || !values.name}>Continue</Button>
          </div>
        </div>
      )}

      {/* Step 3: Review & Deploy */}
      {step === 'deploy' && createJob && selectedBlueprint && (
        <DeployStep
          blueprint={selectedBlueprint}
          job={createJob}
          values={values}
          infra={infra}
          entityLabel={entityLabel}
          deploy={deploy}
          capacity={capacity}
          provisionEstimate={provisionEstimate}
          provisionPriceFormatted={provisionPriceFormatted}
          hasProvisionRfq={hasProvisionRfq}
          priceLoading={priceLoading}
          serviceInfo={serviceInfo}
          serviceValidating={serviceValidating}
          serviceError={serviceError}
          onBack={() => { setStep('configure'); deploy.reset(); }}
          onDeploy={deploy.deploy}
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

// ── Instance Infra Bar ──

function InstanceInfraBar({
  infra, operators, operatorsLoading, hasValidService, onOpenModal,
}: {
  infra: { blueprintId: string; serviceId: string };
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  hasValidService: boolean;
  onOpenModal: () => void;
}) {
  return (
    <div className="glass-card rounded-lg p-3 flex items-center justify-between mb-6">
      <div className="flex items-center gap-4">
        <div className="flex items-center gap-2">
          <div className="i-ph:cube text-sm text-cloud-elements-textTertiary" />
          <span className="text-xs text-cloud-elements-textTertiary">Blueprint</span>
          <Badge variant="accent">#{infra.blueprintId}</Badge>
        </div>
        <div className="flex items-center gap-2">
          <div className="i-ph:users-three text-sm text-cloud-elements-textTertiary" />
          <span className="text-xs text-cloud-elements-textTertiary">
            {operatorsLoading ? 'Discovering...' : `${operators.length} operators`}
          </span>
        </div>
        {hasValidService && (
          <div className="flex items-center gap-2">
            <div className="i-ph:check-circle text-sm text-teal-400" />
            <span className="text-xs text-teal-400">Service #{infra.serviceId}</span>
          </div>
        )}
      </div>
      <Button variant="ghost" size="sm" onClick={onOpenModal}>Change</Button>
    </div>
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
  deploy: ReturnType<typeof useCreateDeploy>;
  capacity?: number | bigint;
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
  blueprint, job, values, infra, entityLabel, deploy,
  capacity, provisionEstimate, provisionPriceFormatted,
  hasProvisionRfq, priceLoading,
  serviceInfo, serviceValidating, serviceError,
  onBack, onDeploy, onViewList, onOpenInfra, onProvisionReady,
}: DeployStepProps) {
  const { address } = useAccount();
  const [showAllJobs, setShowAllJobs] = useState(false);

  const name = String(values.name || '');
  const image = String(values.image || '');
  const cpuCores = Number(values.cpuCores) || 2;
  const memoryMb = Number(values.memoryMb) || 2048;
  const diskGb = Number(values.diskGb) || 10;
  const costDisplay = hasProvisionRfq ? provisionPriceFormatted : `~${formatCost(provisionEstimate)}`;
  const { status, txHash, error, isNewService, isInstanceMode, hasValidService, operators, operatorsLoading, provision, callId } = deploy;
  const isSandbox = !isInstanceMode;
  const isActive = status !== 'idle';
  const isComplete = status === 'confirmed' || status === 'ready';

  // Separate config fields into key vs advanced
  const visibleFields = job.fields.filter((f) => !f.internal);
  const keyFieldNames = new Set(['name', 'image', 'stack', 'cpuCores', 'memoryMb', 'diskGb']);
  const extraFields = visibleFields.filter((f) => !keyFieldNames.has(f.name));
  const activeExtras = extraFields.filter((f) => {
    const v = values[f.name];
    if (f.type === 'boolean') return !!v;
    return v != null && v !== '' && v !== '{}' && v !== f.defaultValue;
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
            <ServiceStatusBadge
              infra={infra}
              serviceInfo={serviceInfo}
              serviceValidating={serviceValidating}
              serviceError={serviceError}
              isInstanceMode={isInstanceMode}
              hasValidService={hasValidService}
            />
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
      {otherJobs.length > 0 && (
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
      )}

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
      {isActive && <TxStatusCard status={status} txHash={txHash} error={error ?? undefined} entityLabel={entityLabel} isNewService={isNewService} />}

      {/* ── Provision Progress ── */}
      {status === 'confirmed' && isSandbox && callId && (
        <ProvisionProgress callId={callId} onReady={onProvisionReady} />
      )}
      {status === 'confirmed' && isInstanceMode && (
        <InstanceProvisionCard provision={provision} />
      )}

      {/* ── Operators (instance mode, new service, idle) ── */}
      {isNewService && status === 'idle' && (
        <OperatorList operators={operators} operatorsLoading={operatorsLoading} blueprintId={infra.blueprintId} />
      )}

      {/* ── Service warning (sandbox mode only) ── */}
      {status === 'idle' && !isInstanceMode && (serviceError || (serviceInfo && (!serviceInfo.active || !serviceInfo.permitted))) && (
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
            <Button variant="secondary" size="sm" onClick={onOpenInfra}>Settings</Button>
          </div>
        </div>
      )}

      {/* ── Wallet warning ── */}
      {!address && status === 'idle' && (
        <div className="flex items-center gap-2 px-1">
          <div className="i-ph:wallet text-sm text-amber-400" />
          <span className="text-xs text-amber-400/80">Connect wallet to deploy</span>
        </div>
      )}

      {/* ── Actions ── */}
      <div className="flex justify-between pt-1">
        <Button variant="secondary" onClick={onBack}>Back</Button>
        {isComplete ? (
          <Button variant="success" onClick={onViewList}>
            <div className="i-ph:check-bold text-sm" />
            View {entityLabel}s
          </Button>
        ) : (
          <DeployButton
            status={status}
            canDeploy={deploy.canDeploy}
            isNewService={isNewService}
            priceLoading={priceLoading}
            serviceValidating={serviceValidating}
            costDisplay={costDisplay}
            onDeploy={onDeploy}
          />
        )}
      </div>
    </div>
  );
}

// ── Sub-components (extracted for readability) ──

function ServiceStatusBadge({
  infra, serviceInfo, serviceValidating, serviceError, isInstanceMode, hasValidService,
}: {
  infra: { serviceId: string };
  serviceInfo: { active: boolean; permitted: boolean } | null;
  serviceValidating: boolean;
  serviceError: string | null;
  isInstanceMode: boolean;
  hasValidService: boolean;
}) {
  if (serviceValidating) {
    return (
      <>
        <div className="w-3 h-3 rounded-full border border-cloud-elements-textTertiary border-t-transparent animate-spin" />
        <span className="text-cloud-elements-textTertiary">Checking service...</span>
      </>
    );
  }
  if (serviceInfo?.active && serviceInfo?.permitted) {
    return (
      <>
        <div className="i-ph:check-circle-fill text-sm text-teal-400" />
        <span className="text-teal-400">Service #{infra.serviceId}</span>
      </>
    );
  }
  if (isInstanceMode && !hasValidService) {
    return (
      <>
        <div className="i-ph:plus-circle text-sm text-violet-400" />
        <span className="text-violet-400">New service</span>
      </>
    );
  }
  if (serviceInfo && !serviceInfo.active) {
    return (
      <>
        <div className="i-ph:x-circle text-sm text-crimson-400" />
        <span className="text-crimson-400">Service #{infra.serviceId} inactive</span>
      </>
    );
  }
  if (serviceInfo && !serviceInfo.permitted) {
    return (
      <>
        <div className="i-ph:warning text-sm text-amber-400" />
        <span className="text-amber-400">Not permitted</span>
      </>
    );
  }
  if (serviceError) {
    return (
      <>
        <div className="i-ph:x-circle text-sm text-crimson-400" />
        <span className="text-crimson-400">Service not found</span>
      </>
    );
  }
  return (
    <>
      <div className="i-ph:globe-simple text-sm text-cloud-elements-textTertiary" />
      <span className="text-cloud-elements-textTertiary">Service #{infra.serviceId}</span>
    </>
  );
}

function TxStatusCard({
  status, txHash, error, entityLabel, isNewService,
}: {
  status: DeployStatus;
  txHash?: `0x${string}`;
  error?: string;
  entityLabel: string;
  isNewService: boolean;
}) {
  const borderClass = status === 'confirmed' ? 'border-teal-500/20 bg-teal-500/[0.03]'
    : status === 'failed' ? 'border-crimson-500/20 bg-crimson-500/[0.03]'
    : 'border-white/[0.06] bg-white/[0.02]';

  const messages: Record<DeployStatus, string> = {
    idle: '',
    signing: isNewService ? 'Confirm service creation in wallet...' : 'Confirm in wallet...',
    pending: isNewService ? 'Creating service on-chain...' : 'Confirming on-chain...',
    confirmed: isNewService ? 'Service created — waiting for operator provisioning' : `${entityLabel} creation confirmed`,
    provisioning: 'Operator provisioning in progress...',
    ready: `${entityLabel} is ready`,
    failed: 'Transaction failed',
  };

  const icons: Record<DeployStatus, React.ReactNode> = {
    idle: null,
    signing: <div className="w-5 h-5 rounded-full border-2 border-amber-400 border-t-transparent animate-spin" />,
    pending: <div className="w-5 h-5 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />,
    confirmed: <div className="i-ph:check-circle-fill text-lg text-teal-400" />,
    provisioning: <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />,
    ready: <div className="i-ph:check-circle-fill text-lg text-teal-400" />,
    failed: <div className="i-ph:x-circle-fill text-lg text-crimson-400" />,
  };

  return (
    <div className={cn('rounded-xl border p-4', borderClass)}>
      <div className="flex items-center gap-3">
        {icons[status]}
        <div className="flex-1 min-w-0">
          <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
            {messages[status]}
          </p>
          {txHash && (
            <p className="text-[11px] font-data text-cloud-elements-textTertiary mt-0.5 truncate">{txHash}</p>
          )}
          {error && <p className="text-xs text-crimson-400 mt-0.5">{error}</p>}
        </div>
      </div>
    </div>
  );
}

function InstanceProvisionCard({ provision }: { provision?: { sandboxId: string; sidecarUrl: string } }) {
  return (
    <div className={cn(
      'rounded-xl border p-4',
      provision ? 'border-teal-500/20 bg-teal-500/[0.03]' : 'border-violet-500/20 bg-violet-500/[0.03]',
    )}>
      <div className="flex items-center gap-3">
        {provision ? (
          <>
            <div className="i-ph:check-circle-fill text-lg text-teal-400" />
            <div>
              <p className="text-sm font-display font-medium text-teal-400">Instance ready</p>
              <p className="text-[11px] text-cloud-elements-textTertiary mt-0.5 font-data truncate max-w-sm">
                {provision.sidecarUrl}
              </p>
            </div>
          </>
        ) : (
          <>
            <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />
            <div>
              <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">Waiting for operator...</p>
              <p className="text-[11px] text-cloud-elements-textTertiary mt-0.5">Watching for on-chain provisioning event</p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function OperatorList({ operators, operatorsLoading, blueprintId }: { operators: DiscoveredOperator[]; operatorsLoading: boolean; blueprintId: string }) {
  return (
    <div className="glass-card rounded-xl p-4">
      <div className="flex items-center gap-2 mb-3">
        <div className="i-ph:users-three text-sm text-cloud-elements-textTertiary" />
        <span className="text-xs font-display font-medium text-cloud-elements-textSecondary">
          Operators ({operatorsLoading ? '...' : operators.length})
        </span>
      </div>
      {operatorsLoading ? (
        <div className="flex items-center gap-2">
          <div className="w-3 h-3 rounded-full border border-cloud-elements-textTertiary border-t-transparent animate-spin" />
          <span className="text-xs text-cloud-elements-textTertiary">Discovering operators for blueprint #{blueprintId}...</span>
        </div>
      ) : operators.length === 0 ? (
        <div className="flex items-center gap-2">
          <div className="i-ph:warning text-sm text-amber-400" />
          <span className="text-xs text-amber-400">No operators found for this blueprint</span>
        </div>
      ) : (
        <div className="space-y-1.5">
          {operators.map((op) => (
            <div key={op.address} className="flex items-center gap-2 py-1">
              <Identicon address={op.address} size={18} />
              <span className="text-xs font-data text-cloud-elements-textSecondary truncate">{op.address}</span>
            </div>
          ))}
          <p className="text-[11px] text-cloud-elements-textTertiary mt-2">
            A new service will be created with these operators. Your sandbox config will be passed as service request inputs.
          </p>
        </div>
      )}
    </div>
  );
}

function DeployButton({
  status, canDeploy, isNewService, priceLoading, serviceValidating, costDisplay, onDeploy,
}: {
  status: DeployStatus;
  canDeploy: boolean;
  isNewService: boolean;
  priceLoading: boolean;
  serviceValidating: boolean;
  costDisplay: string;
  onDeploy: () => void;
}) {
  const isBusy = status === 'signing' || status === 'pending';
  const isDisabled = !canDeploy || isBusy || priceLoading || serviceValidating;

  return (
    <Button size="lg" onClick={onDeploy} disabled={isDisabled}>
      {isBusy ? (
        <>
          <div className="w-4 h-4 rounded-full border-2 border-white/40 border-t-white animate-spin" />
          {status === 'signing' ? 'Confirm in wallet...' : 'Deploying...'}
        </>
      ) : priceLoading ? (
        'Loading price...'
      ) : isNewService ? (
        <>
          <div className="i-ph:lightning text-base" />
          Create Service & Deploy
        </>
      ) : (
        <>
          <div className="i-ph:lightning text-base" />
          Deploy for {costDisplay}
        </>
      )}
    </Button>
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
