import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { Input, Select } from '@tangle-network/blueprint-ui/components';
import { InfrastructureModal, InfraBar } from '~/components/shared/InfrastructureModal';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { infraStore, updateInfra } from '@tangle-network/blueprint-ui';
import { BlueprintJobForm, type FormSection } from '@tangle-network/blueprint-ui/components';
import { Identicon } from '@tangle-network/blueprint-ui/components';

import { useJobForm } from '@tangle-network/blueprint-ui';
import { useJobPrice } from '@tangle-network/blueprint-ui';
import { useServiceValidation } from '@tangle-network/blueprint-ui';
import { formatCost } from '@tangle-network/blueprint-ui';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { useCreateDeploy, type DeployStatus } from '~/lib/hooks/useCreateDeploy';
import { getAllBlueprints, getBlueprint, type BlueprintDefinition, type JobDefinition } from '@tangle-network/blueprint-ui';
import { updateSandboxStatus } from '~/lib/stores/sandboxes';
import { updateInstanceStatus } from '~/lib/stores/instances';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { BlueprintBadgeInline } from '~/components/shared/InfraSummaryBits';
import type { DiscoveredOperator } from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';
import { EnvEditor } from '~/components/shared/EnvEditor';
import {
  BUNDLED_AGENT_OPTIONS,
  BUNDLED_NO_AGENT_VALUE,
  isBundledSandboxImage,
  normalizeAgentIdentifier,
  sanitizeBundledAgentIdentifier,
} from '~/lib/agents';

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

const SANDBOX_PRE_AGENT_SECTIONS: FormSection[] = [
  { label: 'Identity', fields: ['name'] },
  { label: 'Image', fields: ['image'] },
];

const SANDBOX_POST_AGENT_SECTIONS: FormSection[] = [
  { label: 'Runtime & Stack', fields: ['runtimeBackend', 'stack'] },
  { label: 'Resources', fields: ['cpuCores', 'memoryMb', 'diskGb'] },
  { label: 'Timeouts', fields: ['maxLifetimeSeconds', 'idleTimeoutSeconds'] },
  { label: 'Features', fields: ['sshEnabled', 'sshPublicKey'] },
  { label: 'Advanced Options', fields: ['metadataJson', 'teeRequired', 'teeType'], collapsed: true },
];

const PROVISION_SECTIONS: FormSection[] = [
  { label: 'Identity', fields: ['name'] },
  { label: 'Image & Stack', fields: ['image', 'runtimeBackend', 'stack'] },
  { label: 'Resources', fields: ['cpuCores', 'memoryMb', 'diskGb'] },
  { label: 'Timeouts', fields: ['maxLifetimeSeconds', 'idleTimeoutSeconds'] },
  { label: 'Features', fields: ['sshEnabled', 'sshPublicKey'] },
  { label: 'Advanced Options', fields: ['metadataJson', 'teeRequired', 'teeType'], collapsed: true },
];

// ── Wizard Steps ──

type WizardStep = 'blueprint' | 'configure' | 'deploy';

const STEPS: { key: WizardStep; label: string; icon: string }[] = [
  { key: 'blueprint', label: 'Blueprint', icon: 'i-ph:cube' },
  { key: 'configure', label: 'Configure', icon: 'i-ph:gear' },
  { key: 'deploy', label: 'Deploy', icon: 'i-ph:lightning' },
];

function parsePortsInput(value: string): number[] {
  return value
    .split(',')
    .map((s) => parseInt(s.trim(), 10))
    .filter((n) => n > 0 && n <= 65535);
}

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

  // Refs for values that should not re-trigger the init effect
  const addressRef = useRef(address);
  addressRef.current = address;
  const validateServiceRef = useRef(validateService);
  validateServiceRef.current = validateService;

  // Auto-set infra for preselected blueprint (from query param).
  // Only fires on mount (preselected is derived from searchParams, which are
  // stable on initial render). Address and validateService are read from refs
  // to avoid re-triggering when the wallet connects after mount.
  useEffect(() => {
    if (preselected) {
      const mapping = BLUEPRINT_INFRA[preselected.id];
      if (mapping) {
        updateInfra({ blueprintId: mapping.blueprintId, serviceId: mapping.serviceId, serviceValidated: false });
        validateServiceRef.current(BigInt(mapping.serviceId), addressRef.current);
      }
    }
  }, [preselected]);

  // Sync service validation result back to infraStore so useCreateDeploy can read it.
  // useServiceValidation stores results in local state; useCreateDeploy reads infra.serviceInfo.
  useEffect(() => {
    if (serviceInfo) {
      updateInfra({
        serviceValidated: true,
        serviceInfo: {
          active: serviceInfo.active,
          operatorCount: serviceInfo.operatorCount,
          owner: serviceInfo.owner,
          blueprintId: String(serviceInfo.blueprintId),
          permitted: serviceInfo.permitted,
        },
      });
    }
  }, [serviceInfo]);

  // The create/provision job: first lifecycle job that doesn't require an existing resource
  const createJob = useMemo<JobDefinition | null>(() => {
    if (!selectedBlueprint) return null;
    return selectedBlueprint.jobs.find(
      (j) => j.category === 'lifecycle' && !j.requiresSandbox,
    ) ?? null;
  }, [selectedBlueprint]);

  const { values, errors, onChange, validate, reset: resetForm } = useJobForm(createJob);

  // Extra ports input (not an ABI field — merged into metadataJson before deploy)
  const [portsInput, setPortsInput] = useState('');
  const runtimeBackend = String(values.runtimeBackend || 'docker').toLowerCase();
  const supportsMetadataPorts = runtimeBackend !== 'firecracker';
  const selectedImage = String(values.image || '');
  const usesBundledAgentSelector = selectedBlueprint?.id === 'ai-agent-sandbox-blueprint' && isBundledSandboxImage(selectedImage);
  const configuredAgentIdentifier = normalizeAgentIdentifier(values.agentIdentifier);
  const isSandboxBlueprint = selectedBlueprint?.id === 'ai-agent-sandbox-blueprint';

  // Keep TEE controls in sync with runtime backend selection.
  useEffect(() => {
    if (runtimeBackend === 'tee' && values.teeRequired !== true) {
      onChange('teeRequired', true);
      return;
    }
    if (runtimeBackend === 'firecracker') {
      if (values.teeRequired) {
        onChange('teeRequired', false);
      }
      if (String(values.teeType ?? '0') !== '0') {
        onChange('teeType', '0');
      }
    }
  }, [runtimeBackend, values.teeRequired, values.teeType, onChange]);

  // Firecracker backend does not support metadata_json.ports in this runtime.
  useEffect(() => {
    if (!supportsMetadataPorts && portsInput.trim().length > 0) {
      setPortsInput('');
    }
  }, [supportsMetadataPorts, portsInput]);

  useEffect(() => {
    if (!usesBundledAgentSelector) return;
    const sanitized = sanitizeBundledAgentIdentifier(values.agentIdentifier);
    if (sanitized === configuredAgentIdentifier) return;
    onChange('agentIdentifier', sanitized);
  }, [usesBundledAgentSelector, values.agentIdentifier, configuredAgentIdentifier, onChange]);

  // Merge runtime backend + ports into metadataJson.
  const mergedValues = useMemo(() => {
    const ports = parsePortsInput(portsInput);

    let metadata: Record<string, unknown> = {};
    try {
      const parsed = JSON.parse(String(values.metadataJson || '{}'));
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        metadata = parsed as Record<string, unknown>;
      }
    } catch {
      metadata = {};
    }

    metadata.runtime_backend = runtimeBackend;
    if (supportsMetadataPorts && ports.length > 0) {
      metadata.ports = ports;
    } else {
      delete metadata.ports;
    }

    const nextValues: Record<string, unknown> = {
      ...values,
      metadataJson: JSON.stringify(metadata),
      // Keep the deprecated ABI field pinned for backward-compatible encoding.
      webTerminalEnabled: true,
    };
    if (runtimeBackend === 'tee') {
      nextValues.teeRequired = true;
    } else if (runtimeBackend === 'firecracker') {
      nextValues.teeRequired = false;
      nextValues.teeType = '0';
    }

    return nextValues;
  }, [runtimeBackend, supportsMetadataPorts, values, portsInput]);

  // Unified deploy hook — manages both submitJob and requestService paths
  const deploy = useCreateDeploy({ blueprint: selectedBlueprint, job: createJob, values: mergedValues, infra, validate });
  const { reset: deployReset } = deploy;

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
    deployReset();
    const mapping = BLUEPRINT_INFRA[bp.id];
    if (mapping) {
      updateInfra({ blueprintId: mapping.blueprintId, serviceId: mapping.serviceId, serviceValidated: false });
      if (mapping.serviceId) {
        validateService(BigInt(mapping.serviceId), address);
      }
    }
    setStep('configure');
  }, [resetForm, deployReset, address, validateService]);

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
              {isSandboxBlueprint ? (
                <>
                  <BlueprintJobForm
                    job={createJob}
                    values={values}
                    onChange={onChange}
                    errors={errors}
                    sections={SANDBOX_PRE_AGENT_SECTIONS}
                  />

                  <AgentConfigurationField
                    image={selectedImage}
                    value={configuredAgentIdentifier}
                    usesBundledSelector={usesBundledAgentSelector}
                    onChange={(next) => onChange('agentIdentifier', next)}
                  />

                  <BlueprintJobForm
                    job={createJob}
                    values={values}
                    onChange={onChange}
                    errors={errors}
                    sections={SANDBOX_POST_AGENT_SECTIONS}
                  />
                </>
              ) : (
                <BlueprintJobForm
                  job={createJob}
                  values={values}
                  onChange={onChange}
                  errors={errors}
                  sections={PROVISION_SECTIONS}
                />
              )}

              {/* Environment variables — key-value editor instead of raw JSON */}
              <div className="mt-6 pt-4 border-t border-cloud-elements-dividerColor space-y-1.5">
                <label className="text-xs font-display font-medium text-cloud-elements-textSecondary">
                  Environment Variables
                </label>
                <EnvEditor
                  value={String(values.envJson || '{}')}
                  onChange={(json) => onChange('envJson', json)}
                />
                <p className="text-[11px] text-cloud-elements-textTertiary">
                  Key-value pairs injected as environment variables into the sandbox.
                </p>
              </div>

              {/* Extra ports — not an ABI field, merged into metadataJson */}
              <div className="mt-6 pt-4 border-t border-cloud-elements-dividerColor space-y-1.5">
                <label className="text-xs font-display font-medium text-cloud-elements-textSecondary">
                  Exposed Ports
                </label>
                <input
                  type="text"
                  value={portsInput}
                  onChange={(e) => setPortsInput(e.target.value)}
                  disabled={!supportsMetadataPorts}
                  placeholder={supportsMetadataPorts ? 'e.g. 3000, 8080, 5432' : 'Not supported for Firecracker runtime'}
                  className={cn(
                    'w-full px-3 py-2 rounded-lg bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus:outline-none focus:border-cloud-elements-borderColorActive transition-colors',
                    !supportsMetadataPorts && 'opacity-60 cursor-not-allowed',
                  )}
                />
                <p className="text-[11px] text-cloud-elements-textTertiary">
                  {supportsMetadataPorts
                    ? 'Comma-separated container ports to expose through the operator API proxy.'
                    : 'Firecracker backend currently does not support metadata_json.ports mappings.'}
                </p>
              </div>
            </CardContent>
          </Card>
          <div className="flex justify-between">
            <Button variant="secondary" onClick={() => setStep('blueprint')}>Back</Button>
            <Button onClick={() => { if (validate()) setStep('deploy'); }} disabled={!createJob || !values.name}>Continue</Button>
          </div>
        </div>
      )}

      {/* Step 3: Review & Deploy */}
      {step === 'deploy' && createJob && selectedBlueprint && (
        <DeployStep
          blueprint={selectedBlueprint}
          job={createJob}
          values={values}
          ports={supportsMetadataPorts ? parsePortsInput(portsInput) : []}
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
          onBack={() => { setStep('configure'); deployReset(); }}
          onDeploy={deploy.deploy}
          onViewList={() => navigate(isSandbox ? '/sandboxes' : '/instances')}
          onOpenInfra={() => setShowInfra(true)}
          onProvisionReady={(sandboxId, sidecarUrl) => {
            if (isSandbox) {
              if (deploy.sandboxDraftKey) {
                updateSandboxStatus(deploy.sandboxDraftKey, 'running', { sandboxId, sidecarUrl });
              }
            } else {
              const name = String(values.name || '');
              updateInstanceStatus(name, 'running', { id: sandboxId, sidecarUrl });
            }
          }}
        />
      )}
    </AnimatedPage>
  );
}

function AgentConfigurationField({
  image,
  value,
  usesBundledSelector,
  onChange,
}: {
  image: string;
  value: string;
  usesBundledSelector: boolean;
  onChange: (value: string) => void;
}) {
  const helpText = usesBundledSelector
    ? 'Choose an agent already bundled in this image. “None” keeps the sandbox compute-only and hides chat.'
    : 'Custom images must already register this agent identifier internally. Typing a new name here does not create a new agent.';
  const selectValue = value || BUNDLED_NO_AGENT_VALUE;

  return (
    <div className="mt-6 pt-4 border-t border-cloud-elements-dividerColor space-y-1.5">
      <label className="text-xs font-display font-medium text-cloud-elements-textSecondary">
        Agent
      </label>
      {usesBundledSelector ? (
        <Select
          value={selectValue}
          onValueChange={(next) => onChange(sanitizeBundledAgentIdentifier(next))}
          options={BUNDLED_AGENT_OPTIONS}
        />
      ) : (
        <Input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={image ? 'default' : 'Choose an image first'}
          className="font-data text-sm"
        />
      )}
      <p className="text-[11px] text-cloud-elements-textTertiary">
        {helpText}
      </p>
    </div>
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
        <BlueprintBadgeInline blueprintId={infra.blueprintId} />
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
                  {Math.min(...bp.jobs.map((j) => j.pricingMultiplier))}x&ndash;{Math.max(...bp.jobs.map((j) => j.pricingMultiplier))}x
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
  ports: number[];
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
  blueprint, job, values, ports, infra, entityLabel, deploy,
  capacity, provisionEstimate, provisionPriceFormatted,
  hasProvisionRfq, priceLoading,
  serviceInfo, serviceValidating, serviceError,
  onBack, onDeploy, onViewList, onOpenInfra, onProvisionReady,
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
    hasValidService,
    operators,
    operatorsLoading,
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
          <ResourcePill icon="i-ph:stack" label={runtimeLabel} />
          <ResourcePill icon="i-ph:cpu" label={`${cpuCores} CPU`} />
          <ResourcePill icon="i-ph:memory" label={`${memoryMb} MB`} />
          <ResourcePill icon="i-ph:hard-drive" label={`${diskGb} GB`} />
          {ports.length > 0 && <ResourcePill icon="i-ph:globe" label={`${ports.length} port${ports.length > 1 ? 's' : ''}`} />}
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
        {(activeExtras.length > 0 || configuredAgentIdentifier) && (
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
            {configuredAgentIdentifier && (
              <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-white/[0.04] text-[11px] font-data text-cloud-elements-textSecondary">
                <div className="i-ph:robot text-[10px] text-teal-400" />
                Agent: {configuredAgentIdentifier}
              </span>
            )}
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

      {status === 'idle' && runtimeBackend === 'firecracker' && (
        <div className="rounded-xl border border-amber-500/20 bg-amber-500/[0.03] p-4">
          <div className="flex items-center gap-3">
            <div className="i-ph:warning-circle text-lg text-amber-400" />
            <div className="flex-1">
              <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                Firecracker requires an operator runtime with Firecracker provisioning enabled
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mt-0.5">
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

      {/* ── Contracts not deployed warning ── */}
      {!contractsDeployed && status === 'idle' && (
        <div className="rounded-xl border border-amber-500/20 bg-amber-500/[0.03] p-4">
          <div className="flex items-center gap-3">
            <div className="i-ph:warning-circle text-lg text-amber-400" />
            <div className="flex-1">
              <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                Contracts not yet deployed on this network
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mt-0.5">
                Please switch to a supported network where the blueprint contracts have been deployed.
              </p>
            </div>
          </div>
        </div>
      )}

      {/* ── Wallet warning ── */}
      {status === 'idle' && (!isConnected || !address) && (
        <div className="flex items-center gap-2 px-1">
          <div className="i-ph:wallet text-sm text-amber-400" />
          <span className="text-xs text-amber-400/80">
            {isReconnecting ? 'Reconnecting wallet...' : 'Connect wallet to deploy'}
          </span>
          {isReconnecting && (
            <div className="w-3 h-3 rounded-full border border-amber-400/40 border-t-amber-400 animate-spin" />
          )}
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
          {error && (
            <div className="mt-1">
              <p className="text-xs text-crimson-400">{error}</p>
              {/resource not available|request already pending/i.test(error) && (
                <p className="text-[11px] text-cloud-elements-textTertiary mt-1">
                  MetaMask may have a pending request. Open MetaMask, dismiss any popups, then try again.
                </p>
              )}
              {/user rejected|denied/i.test(error) && (
                <p className="text-[11px] text-cloud-elements-textTertiary mt-1">
                  Transaction was rejected in the wallet.
                </p>
              )}
              {/chain.*mismatch|wrong.*network/i.test(error) && (
                <p className="text-[11px] text-cloud-elements-textTertiary mt-1">
                  Your wallet is on a different network. Switch to the correct chain and try again.
                </p>
              )}
            </div>
          )}
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
