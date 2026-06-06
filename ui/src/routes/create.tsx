import { useState, useCallback, useMemo, useEffect, useLayoutEffect, useRef, type ButtonHTMLAttributes, type InputHTMLAttributes, type RefObject, type ReactNode, type TextareaHTMLAttributes } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { InfrastructureModal } from '~/components/shared/InfrastructureModal';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { infraStore, updateInfra } from '@tangle-network/blueprint-ui';
import {
  ConsoleChip,
  ConsolePage,
  ConsoleSection,
  type ConsoleMetric,
} from '~/components/console/ConsolePrimitives';

import { useJobForm } from '@tangle-network/blueprint-ui';
import { useJobPrice } from '@tangle-network/blueprint-ui';
import { useServiceValidation } from '@tangle-network/blueprint-ui';
import { formatCost } from '@tangle-network/blueprint-ui';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { useCreateDeploy, type DeployStatus } from '~/lib/hooks/useCreateDeploy';
import { getAllBlueprints, getBlueprint, type BlueprintDefinition, type JobDefinition, type JobFieldDef } from '@tangle-network/blueprint-ui';
import { updateSandboxStatus } from '~/lib/stores/sandboxes';
import { updateInstanceStatus } from '~/lib/stores/instances';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import type { DiscoveredOperator } from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';
import { EnvEditor } from '~/components/shared/EnvEditor';
import { ConnectWalletPanel } from '~/components/shared/ConnectWalletPanel';
import { truncateAddress } from '~/lib/utils/truncate-address';
import {
  IdentityMark,
  OperatorIdentity,
  getAgentIdentity,
  getBlueprintIdentity,
  getCapabilityIdentity,
  getImageIdentity,
  getOperatorIdentity,
  getResourceIdentity,
  getRuntimeIdentity,
  getStackIdentity,
  type IdentityMeta,
} from '~/components/shared/VisualIdentity';
import {
  BUNDLED_AGENT_OPTIONS,
  BUNDLED_NO_AGENT_VALUE,
  isBundledSandboxImage,
  normalizeAgentIdentifier,
  sanitizeBundledAgentIdentifier,
} from '~/lib/agents';
import {
  INSTANCE_ONCHAIN_BLUEPRINT_ID,
  INSTANCE_ONCHAIN_SERVICE_ID,
  SANDBOX_ONCHAIN_BLUEPRINT_ID,
  SANDBOX_ONCHAIN_SERVICE_ID,
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
  TEE_INSTANCE_ONCHAIN_SERVICE_ID,
} from '~/lib/config';

type ConsoleTone = NonNullable<ConsoleMetric['tone']>;

// ── Blueprint → on-chain ID mapping from env vars ──

const BLUEPRINT_INFRA: Record<string, { blueprintId: string; serviceId: string }> = {
  'ai-agent-sandbox-blueprint': {
    blueprintId: SANDBOX_ONCHAIN_BLUEPRINT_ID,
    serviceId: SANDBOX_ONCHAIN_SERVICE_ID,
  },
  'ai-agent-instance-blueprint': {
    blueprintId: INSTANCE_ONCHAIN_BLUEPRINT_ID,
    serviceId: INSTANCE_ONCHAIN_SERVICE_ID,
  },
  'ai-agent-tee-instance-blueprint': {
    blueprintId: TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
    serviceId: TEE_INSTANCE_ONCHAIN_SERVICE_ID,
  },
};

// ── Wizard Steps ──

type WizardStep = 'blueprint' | 'configure' | 'deploy';
type ServiceSetupMode = 'existing' | 'new';
type LaunchSelectOption = { label: string; value: string; detail?: string; identity?: IdentityMeta };
const CUSTOM_IMAGE_VALUE = '__custom_image__';

function parsePortsInput(value: string): number[] {
  return value
    .split(',')
    .map((s) => parseInt(s.trim(), 10))
    .filter((n) => n > 0 && n <= 65535);
}

function parseCapabilitiesJson(value: unknown): Set<string> {
  if (Array.isArray(value)) {
    return new Set(value.filter((item): item is string => typeof item === 'string'));
  }
  try {
    const parsed = JSON.parse(String(value || '[]'));
    return Array.isArray(parsed)
      ? new Set(parsed.filter((item): item is string => typeof item === 'string'))
      : new Set();
  } catch {
    return new Set();
  }
}

function setCapabilityJson(value: unknown, capability: string, enabled: boolean): string {
  const capabilities = parseCapabilitiesJson(value);
  if (enabled) {
    capabilities.add(capability);
  } else {
    capabilities.delete(capability);
  }
  return JSON.stringify(Array.from(capabilities).sort());
}

function formatCapacityValue(value: number | bigint | undefined) {
  if (value == null) return '--';
  return typeof value === 'bigint' ? value.toString() : String(value);
}

function runtimeLabel(value: string) {
  if (value === 'firecracker') return 'Firecracker';
  if (value === 'tee') return 'TEE';
  return 'Docker';
}

function field(job: JobDefinition | null, name: string): JobFieldDef | undefined {
  return job?.fields.find((item) => item.name === name);
}

function fieldOptions(job: JobDefinition | null, name: string): { label: string; value: string }[] {
  return field(job, name)?.options ?? [];
}

function valueString(values: Record<string, unknown>, name: string, fallback = ''): string {
  const value = values[name];
  if (value === undefined || value === null) return fallback;
  return String(value);
}

function valueNumber(values: Record<string, unknown>, name: string, fallback: number): number {
  const raw = Number(values[name]);
  return Number.isFinite(raw) ? raw : fallback;
}

function clampNumber(value: number, min?: number, max?: number): number {
  if (typeof min === 'number' && value < min) return min;
  if (typeof max === 'number' && value > max) return max;
  return value;
}

function formatImageOptionLabel(value: string, fallback: string) {
  const image = value.toLowerCase();
  if (image.includes('blueprint-sidecar')) {
    const tag = value.includes(':') ? value.split(':').pop() : '';
    return tag ? `Tangle sidecar: ${tag}` : 'Tangle sidecar';
  }
  if (image.startsWith('ghcr.io/tangle-network/')) {
    return value.replace(/^ghcr\.io\/tangle-network\//, 'Tangle image: ');
  }
  if (image.startsWith('ghcr.io/')) {
    return value.replace(/^ghcr\.io\//, 'GHCR: ');
  }
  return fallback;
}

function hoursFromSeconds(value: unknown, fallbackSeconds: number): number {
  const seconds = Number(value);
  return Math.max(0, Math.round((Number.isFinite(seconds) ? seconds : fallbackSeconds) / 3600));
}

function minutesFromSeconds(value: unknown, fallbackSeconds: number): number {
  const seconds = Number(value);
  return Math.max(0, Math.round((Number.isFinite(seconds) ? seconds : fallbackSeconds) / 60));
}

function serviceTone({
  serviceValidating,
  serviceError,
  hasValidService,
  isNewService,
}: {
  serviceValidating: boolean;
  serviceError: string | null;
  hasValidService: boolean;
  isNewService: boolean;
}): ConsoleTone {
  if (serviceError) return 'danger';
  if (serviceValidating) return 'warn';
  if (hasValidService || isNewService) return 'ready';
  return 'muted';
}

export default function CreatePage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { address, isConnected, status: walletStatus } = useAccount();
  const isReconnectingWallet = walletStatus === 'reconnecting';
  const infra = useStore(infraStore);
  const { validate: validateService, isValidating: serviceValidating, serviceInfo, error: serviceError } = useServiceValidation();
  const { data: capacity } = useAvailableCapacity();

  // Pre-select from query params
  const preselectedId = searchParams.get('blueprint');
  const preselected = preselectedId ? getBlueprint(preselectedId) : undefined;

  const [selectedBlueprint, setSelectedBlueprint] = useState<BlueprintDefinition | undefined>(preselected);
  const [step, setStep] = useState<WizardStep>(preselected ? 'configure' : 'blueprint');
  const [showInfra, setShowInfra] = useState(false);
  const [infraMode, setInfraMode] = useState<ServiceSetupMode>('existing');
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [attemptedContinue, setAttemptedContinue] = useState(false);
  const nameInputRef = useRef<HTMLInputElement>(null);
  const requestedServiceMode = searchParams.get('serviceMode');

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

  useEffect(() => {
    if (requestedServiceMode !== 'new') return;
    setInfraMode('new');
    setShowInfra(true);
  }, [requestedServiceMode]);

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

  const isTeeBlueprint = selectedBlueprint?.id === 'ai-agent-tee-instance-blueprint';

  // For non-TEE blueprints, hide the TEE runtime backend option from the form.
  const displayJob = useMemo<JobDefinition | null>(() => {
    if (!createJob || isTeeBlueprint) return createJob;
    return {
      ...createJob,
      fields: createJob.fields.map((f) =>
        f.name === 'runtimeBackend' && f.options
          ? { ...f, options: f.options.filter((o) => o.value !== 'tee') }
          : f,
      ),
    };
  }, [createJob, isTeeBlueprint]);

  // Extra ports input (not an ABI field; merged into metadataJson before deploy)
  const [portsInput, setPortsInput] = useState('');
  const capabilities = parseCapabilitiesJson(values.capabilitiesJson);
  const allHarnessEnabled = capabilities.has('all_harness');
  const computerUseEnabled = capabilities.has('computer_use');
  const runtimeBackend = String(values.runtimeBackend || 'docker').toLowerCase();
  const selectedImage = String(values.image || '');
  const supportsAgentConfiguration = !!createJob?.fields.some((field) => field.name === 'agentIdentifier');
  const usesBundledAgentSelector = supportsAgentConfiguration && isBundledSandboxImage(selectedImage);
  const configuredAgentIdentifier = normalizeAgentIdentifier(values.agentIdentifier);

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
    if (ports.length > 0) {
      metadata.ports = ports;
    } else {
      delete metadata.ports;
    }

    const nextCapabilities = Array.from(parseCapabilitiesJson(values.capabilitiesJson)).sort();

    const nextValues: Record<string, unknown> = {
      ...values,
      metadataJson: JSON.stringify(metadata),
      capabilitiesJson: JSON.stringify(nextCapabilities),
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
  }, [runtimeBackend, values, portsInput]);

  // Unified deploy hook — manages both submitJob and requestService paths
  const deploy = useCreateDeploy({ blueprint: selectedBlueprint, job: createJob, values: mergedValues, infra, validate, capacity });
  const { reset: deployReset } = deploy;

  const isSandbox = deploy.mode === 'sandbox';
  const entityLabel = isSandbox ? 'Sandbox' : 'Instance';
  const blueprints = useMemo(() => getAllBlueprints(), []);

  // Per-job RFQ pricing
  const operatorRpcUrl = infra.serviceInfo?.operators?.[0]?.rpcAddress;
  const blueprintId = BigInt(infra.blueprintId || '0');
  const serviceIdBig = BigInt(infra.serviceId || '0');
  // `requester` is the wallet that will execute the job — tnt-core v0.13.0
  // binds quotes to this address so the operator quote can scope pricing
  // (per-account rate-limits, holder discounts, etc.) to the actual caller.
  // The `enabled` flag also gates on `!!address` so we don't query with the
  // zero-address sentinel by accident.
  const ZERO_ADDR = '0x0000000000000000000000000000000000000000' as const;
  const { quote: provisionQuote, isLoading: priceLoading, formattedPrice: provisionPriceFormatted } = useJobPrice(
    operatorRpcUrl,
    serviceIdBig,
    createJob?.id ?? 0,
    blueprintId,
    step === 'deploy' && !!operatorRpcUrl && serviceIdBig > 0n && !!createJob && !!address,
    (address ?? ZERO_ADDR) as `0x${string}`,
  );
  const provisionEstimate = BigInt(createJob?.pricingMultiplier ?? 50) * 1_000_000_000_000_000n;
  const hasProvisionRfq = !!provisionQuote;

  // ── Handlers ──

  const handleSelectBlueprint = useCallback((bp: BlueprintDefinition) => {
    setSelectedBlueprint(bp);
    resetForm();
    deployReset();
    setAttemptedContinue(false);
    const mapping = BLUEPRINT_INFRA[bp.id];
    if (mapping) {
      updateInfra({ blueprintId: mapping.blueprintId, serviceId: mapping.serviceId, serviceValidated: false });
      if (mapping.serviceId) {
        validateService(BigInt(mapping.serviceId), address);
      }
    }
    setStep('configure');
  }, [resetForm, deployReset, address, validateService]);

  const handleContinue = useCallback(() => {
    setAttemptedContinue(true);
    if (!String(values.name || '').trim()) {
      nameInputRef.current?.focus();
      return;
    }
    if (validate()) setStep('deploy');
  }, [validate, values.name]);

  const openInfra = useCallback((mode: ServiceSetupMode = 'existing') => {
    setInfraMode(mode);
    setShowInfra(true);
  }, []);

  const showConnectPanel = !isConnected && !address && !isReconnectingWallet;
  const parsedPorts = parsePortsInput(portsInput);

  return (
    <ConsolePage
      title="Launch Workspace"
      eyebrow="Tangle agent compute"
      actions={step !== 'blueprint' ? (
        <LaunchActionButton variant="secondary" onClick={() => openInfra('existing')}>
          <span className="i-ph:sliders-horizontal text-base" />
          {step === 'deploy' ? 'Service settings' : 'Infrastructure'}
        </LaunchActionButton>
      ) : null}
    >
      <div className={cn('grid min-h-full gap-5', step !== 'deploy' && 'xl:grid-cols-[minmax(0,1fr)_360px]')}>
        <main className="min-w-0 space-y-5">
          {showConnectPanel && step !== 'deploy' && (
            <ConnectWalletPanel
              description="Provisioning a sandbox or instance requires a connected wallet on Tangle Network. You can browse blueprints below, but deploying will be blocked until you connect."
            />
          )}

          {step !== 'deploy' ? (
            <LaunchModeStrip
              blueprints={blueprints}
              selectedBlueprint={selectedBlueprint}
              onSelect={handleSelectBlueprint}
            />
          ) : null}

          {step === 'blueprint' && (
            <ConsoleSection title="Next">
              <div className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:justify-between">
                <p className="max-w-xl text-sm text-[var(--sandbox-console-muted)]">
                  Choose the provisioning mode above. Sandbox mode is for shared cloud capacity; Instance and TEE Instance create dedicated service paths.
                </p>
                <LaunchActionButton
                  onClick={() => {
                    const nextBlueprint = selectedBlueprint ?? blueprints[0];
                    if (nextBlueprint) handleSelectBlueprint(nextBlueprint);
                  }}
                >
                  <span className="i-ph:arrow-right text-base" />
                  Continue
                </LaunchActionButton>
              </div>
            </ConsoleSection>
          )}

          {step === 'configure' && createJob && displayJob && (
            <div className="space-y-5">
              <LaunchSpecComposer
                blueprint={selectedBlueprint}
                job={displayJob}
                values={values}
                errors={errors}
                entityLabel={entityLabel}
                nameInputRef={nameInputRef}
                attemptedContinue={attemptedContinue}
                runtimeBackend={runtimeBackend}
                selectedImage={selectedImage}
                supportsAgentConfiguration={supportsAgentConfiguration}
                usesBundledAgentSelector={usesBundledAgentSelector}
                configuredAgentIdentifier={configuredAgentIdentifier}
                allHarnessEnabled={allHarnessEnabled}
                computerUseEnabled={computerUseEnabled}
                portsInput={portsInput}
                isTeeBlueprint={isTeeBlueprint}
                onChange={onChange}
                onPortsChange={setPortsInput}
                onAdvancedOpen={() => setShowAdvanced(true)}
              />
              <div className="flex justify-between gap-3">
                <LaunchActionButton variant="secondary" onClick={() => setStep('blueprint')}>Back</LaunchActionButton>
                <LaunchActionButton onClick={handleContinue} disabled={!createJob}>Continue</LaunchActionButton>
              </div>
            </div>
          )}

          {step === 'deploy' && createJob && displayJob && selectedBlueprint && (
            <DeployStep
              blueprint={selectedBlueprint}
              job={displayJob}
              values={values}
              ports={parsedPorts}
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
              onCreateService={() => openInfra('new')}
              onOpenInfra={() => openInfra('existing')}
              onOpenOperators={() => navigate('/operators')}
              onViewDetail={() => {
                const key = isSandbox
                  ? deploy.sandboxDraftKey
                  : String(values.name || '');
                if (key) navigate(`/${isSandbox ? 'sandboxes' : 'instances'}/${encodeURIComponent(key)}`);
                else navigate(isSandbox ? '/sandboxes' : '/instances');
              }}
              onProvisionReady={(sandboxId, sidecarUrl) => {
                if (isSandbox) {
                  if (deploy.sandboxDraftKey) {
                    updateSandboxStatus(deploy.sandboxDraftKey, 'running', { sandboxId, sidecarUrl });
                  }
                } else {
                  const name = String(values.name || '');
                  updateInstanceStatus(name, 'running', { sandboxId, sidecarUrl });
                }
              }}
            />
          )}
        </main>

        {step !== 'deploy' ? (
          <LaunchSummaryPanel
            step={step}
            selectedBlueprint={selectedBlueprint}
            entityLabel={entityLabel}
            runtimeBackend={runtimeBackend}
            infra={infra}
            capacity={capacity}
            isConnected={isConnected}
            isReconnectingWallet={isReconnectingWallet}
            hasValidService={deploy.hasValidService}
            isNewService={deploy.isNewService}
            serviceValidating={serviceValidating}
            serviceError={serviceError}
            operatorsCount={deploy.operators.length}
            operatorCount={deploy.operatorCount}
            operatorsLoading={deploy.operatorsLoading}
            operatorsError={deploy.operatorsError}
            agentIdentifier={configuredAgentIdentifier}
            ports={parsedPorts}
            onOpenInfra={() => openInfra('existing')}
          />
        ) : null}
      </div>

      {createJob && displayJob ? (
        <AdvancedOptionsModal
          open={showAdvanced}
          onOpenChange={setShowAdvanced}
          job={displayJob}
          values={values}
          runtimeBackend={runtimeBackend}
          isTeeBlueprint={isTeeBlueprint}
          onChange={onChange}
        />
      ) : null}

      <InfrastructureModal open={showInfra} onOpenChange={setShowInfra} initialMode={infraMode} />
    </ConsolePage>
  );
}

function LaunchActionButton({
  variant = 'primary',
  size = 'md',
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'secondary' | 'danger' | 'success';
  size?: 'sm' | 'md' | 'lg';
}) {
  return (
    <button
      type="button"
      className={cn(
        'inline-flex items-center justify-center gap-2 rounded-[5px] border font-display font-bold transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50',
        size === 'sm' && 'h-9 px-3 text-sm',
        size === 'md' && 'h-11 px-4 text-[15px]',
        size === 'lg' && 'h-12 px-5 text-[15px]',
        variant === 'primary' && 'border-[var(--sandbox-console-brand-border)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--sandbox-console-brand)_22%,var(--sandbox-console-panel-strong)),var(--sandbox-console-brand-soft))] text-[var(--sandbox-console-text)] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] hover:border-[var(--sandbox-console-brand)] hover:bg-[rgba(142,89,255,0.26)] hover:shadow-[0_0_0_3px_rgba(168,123,255,0.13),inset_0_1px_0_rgba(255,255,255,0.08)]',
        variant === 'secondary' && 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] shadow-[var(--sandbox-console-control-shadow)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] hover:text-[var(--sandbox-console-text)]',
        variant === 'success' && 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)] hover:bg-[rgba(56,178,172,0.20)]',
        variant === 'danger' && 'border-red-400/20 bg-red-400/10 text-[var(--sandbox-console-danger)] hover:bg-red-400/15',
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

function LaunchModeStrip({
  blueprints,
  selectedBlueprint,
  onSelect,
}: {
  blueprints: BlueprintDefinition[];
  selectedBlueprint?: BlueprintDefinition;
  onSelect: (bp: BlueprintDefinition) => void;
}) {
  return (
    <ConsoleSection title="Launch Mode">
      <div className="grid gap-px bg-[var(--sandbox-console-border)] p-px lg:grid-cols-3">
        {blueprints.map((bp) => {
          const active = selectedBlueprint?.id === bp.id;
          const recommended = bp.id === 'ai-agent-sandbox-blueprint';
          const identity = getBlueprintIdentity(bp.id);

          return (
            <button
              key={bp.id}
              type="button"
              onClick={() => onSelect(bp)}
              className={cn(
                'group min-h-32 bg-[var(--sandbox-console-panel)] p-5 text-left transition-[background-color,box-shadow,transform] duration-150 hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[inset_0_3px_0_var(--sandbox-console-border-hover)] active:scale-[0.995]',
                active && 'bg-[var(--sandbox-console-brand-soft)] shadow-[inset_0_3px_0_var(--sandbox-console-brand)]',
              )}
            >
              <div className="flex items-start justify-between gap-3">
                <IdentityMark identity={identity} size="lg" className="transition-transform duration-150 group-hover:-translate-y-0.5" />
                {recommended ? <ConsoleChip tone="ready">recommended</ConsoleChip> : null}
              </div>
              <div className="mt-3">
                <p className="font-display text-lg font-bold tracking-tight text-[var(--sandbox-console-text)]">{bp.name}</p>
                <p className="mt-1 line-clamp-2 text-sm leading-6 text-[var(--sandbox-console-muted)] group-hover:text-[var(--sandbox-console-secondary)]">{bp.description}</p>
              </div>
            </button>
          );
        })}
      </div>
    </ConsoleSection>
  );
}

function LaunchField({
  label,
  detail,
  error,
  children,
}: {
  label: string;
  detail?: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <label className="block min-w-0 space-y-2">
      <span className="flex items-center justify-between gap-3">
        <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">{label}</span>
        {detail ? <span className="font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">{detail}</span> : null}
      </span>
      {children}
      {error ? <span className="block text-sm text-[var(--sandbox-console-danger)]">{error}</span> : null}
    </label>
  );
}

const launchControlClass = 'min-h-11 w-full rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3.5 py-2.5 font-data text-[15px] font-medium text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow)] placeholder:text-[var(--sandbox-console-subtle)] transition-[background-color,border-color,box-shadow,color] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] focus:border-[var(--sandbox-console-brand-border)] focus:bg-[var(--sandbox-console-control-hover)] focus:shadow-[var(--sandbox-console-control-shadow-focus)] focus:outline-none disabled:cursor-not-allowed disabled:opacity-60';

function LaunchInput({
  label,
  detail,
  error,
  inputRef,
  className,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'ref'> & {
  label: string;
  detail?: string;
  error?: string;
  inputRef?: RefObject<HTMLInputElement>;
}) {
  return (
    <LaunchField label={label} detail={detail} error={error}>
      <input ref={inputRef} aria-label={label} className={cn(launchControlClass, className)} {...props} />
    </LaunchField>
  );
}

function LaunchTextArea({
  label,
  detail,
  error,
  className,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  label: string;
  detail?: string;
  error?: string;
}) {
  return (
    <LaunchField label={label} detail={detail} error={error}>
      <textarea aria-label={label} className={cn(launchControlClass, 'min-h-24 resize-y', className)} {...props} />
    </LaunchField>
  );
}

function LaunchNativeSelect({
  label,
  detail,
  value,
  options,
  onChange,
  disabled,
}: {
  label: string;
  detail?: string;
  value: string;
  options: LaunchSelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<'down' | 'up'>('down');
  const rootRef = useRef<HTMLDivElement>(null);
  const selected = options.find((option) => option.value === value);
  const isDisabled = disabled || options.length === 0;

  useEffect(() => {
    if (!open) return;

    function onPointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') setOpen(false);
    }

    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  useLayoutEffect(() => {
    if (!open) return;
    const rect = rootRef.current?.getBoundingClientRect();
    if (!rect) return;

    const estimatedMenuHeight = Math.min(288, (options.length * 56) + 12);
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    setPlacement(spaceBelow < estimatedMenuHeight + 12 && spaceAbove > spaceBelow ? 'up' : 'down');
  }, [open, options.length]);

  return (
    <div ref={rootRef} className="relative space-y-2">
      <span className="flex items-center justify-between gap-2">
        <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">{label}</span>
        {detail ? <span className="font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">{detail}</span> : null}
      </span>
      <button
        type="button"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={isDisabled}
        onClick={() => setOpen((current) => !current)}
        className={cn(
          'group flex min-h-11 w-full items-center justify-between gap-3 rounded-[5px] border px-3.5 py-2.5 text-left font-data text-[15px] font-medium shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color] duration-150 disabled:cursor-not-allowed disabled:opacity-60',
          open
            ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-control-hover)] text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow-focus)]'
            : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-text)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]',
        )}
      >
        {selected ? (
          <SelectOptionVisual option={selected} />
        ) : (
          <span className="min-w-0 truncate">{value || 'Select option'}</span>
        )}
        <span className={cn('i-ph:caret-down shrink-0 text-sm text-[var(--sandbox-console-muted)] transition-transform group-hover:text-[var(--sandbox-console-text)]', open && 'rotate-180 text-[var(--sandbox-console-brand)]')} />
      </button>
      {open ? (
        <div
          role="listbox"
          aria-label={label}
          className={cn(
            'absolute left-0 right-0 z-[70] max-h-72 overflow-y-auto rounded-[5px] border border-[var(--sandbox-console-menu-border)] bg-[var(--sandbox-console-menu)] p-1.5 shadow-[var(--sandbox-console-menu-shadow)]',
            placement === 'up' ? 'bottom-full mb-2' : 'top-full mt-2',
          )}
        >
          {options.map((option) => {
            const active = option.value === value;
            return (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={active}
                aria-label={option.label}
                onClick={() => {
                  onChange(option.value);
                  setOpen(false);
                }}
                className={cn(
                  'flex w-full items-center justify-between gap-3 rounded-[4px] px-3 py-2.5 text-left font-display text-[15px] font-semibold transition-[background-color,color,box-shadow] duration-150',
                  active
                    ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                    : 'text-[var(--sandbox-console-secondary)] hover:bg-[var(--sandbox-console-menu-strong)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-border-hover)]',
                )}
              >
                <SelectOptionVisual option={option} />
                {active ? <span className="i-ph:check-bold shrink-0 text-xs text-[var(--sandbox-console-brand)]" /> : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function SelectOptionVisual({ option }: { option: LaunchSelectOption }) {
  if (!option.identity) {
    return <span className="min-w-0 truncate">{option.label}</span>;
  }

  return (
    <span className="flex min-w-0 items-center gap-3">
      <IdentityMark identity={option.identity} size="sm" />
      <span className="min-w-0">
        <span className="block truncate">{option.label}</span>
        {(option.detail ?? option.identity.detail) ? (
          <span className="mt-0.5 block truncate font-data text-[11px] font-medium text-[var(--sandbox-console-subtle)]">
            {option.detail ?? option.identity.detail}
          </span>
        ) : null}
      </span>
    </span>
  );
}

function LaunchImageSelect({
  value,
  options,
  onChange,
  placeholder,
}: {
  value: string;
  options: { label: string; value: string }[];
  onChange: (value: string) => void;
  placeholder: string;
}) {
  const selectedOption = options.find((option) => option.value === value);
  const selectOptions = [
    ...options.map((option) => ({
      ...option,
      label: formatImageOptionLabel(option.value, option.label),
      detail: getImageIdentity(option.value).detail,
      identity: getImageIdentity(option.value),
    })),
    {
      label: 'Custom image...',
      value: CUSTOM_IMAGE_VALUE,
      detail: getImageIdentity(CUSTOM_IMAGE_VALUE).detail,
      identity: getImageIdentity(CUSTOM_IMAGE_VALUE),
    },
  ];
  const selectValue = selectedOption ? selectedOption.value : CUSTOM_IMAGE_VALUE;

  return (
    <div className="space-y-2">
      <LaunchNativeSelect
        label="Docker Image"
        value={selectValue}
        options={selectOptions}
        onChange={(next) => {
          if (next === CUSTOM_IMAGE_VALUE) {
            if (selectedOption) onChange('');
            return;
          }
          onChange(next);
        }}
      />
      {selectValue === CUSTOM_IMAGE_VALUE ? (
        <LaunchInput
          label="Custom Image"
          value={selectedOption ? '' : value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          className="font-data"
        />
      ) : null}
    </div>
  );
}

function SegmentedControl({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: { label: string; value: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <div className="space-y-2">
      <p className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">{label}</p>
      <div
        className={cn(
          'grid gap-1 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] p-1 shadow-[var(--sandbox-console-control-shadow)]',
          options.length === 2 ? 'sm:grid-cols-2' : 'sm:grid-cols-3',
        )}
      >
        {options.map((option) => {
          const active = option.value === value;
          const identity = getRuntimeIdentity(option.value);
          return (
            <button
              key={option.value}
              type="button"
              onClick={() => onChange(option.value)}
              className={cn(
                'flex min-h-12 items-center justify-center gap-2 rounded-[4px] px-3 text-center font-display text-sm font-bold transition-[background-color,color,box-shadow,transform] duration-150 active:scale-[0.98]',
                active
                  ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_0_0_0_1px_var(--sandbox-console-brand-border),inset_0_3px_0_var(--sandbox-console-brand)]'
                  : 'text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_0_3px_0_var(--sandbox-console-border-hover)]',
              )}
            >
              <IdentityMark identity={identity} size="sm" />
              <span className="whitespace-nowrap text-[13px] sm:text-sm">{option.label.replace(' (default)', '')}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function LaunchToggle({
  label,
  detail,
  identity,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  detail?: string;
  identity?: IdentityMeta;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'group flex w-full items-center gap-3 rounded-[5px] border p-3.5 text-left shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.99] disabled:cursor-not-allowed disabled:opacity-60',
        checked
          ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]',
      )}
    >
      {identity ? <IdentityMark identity={identity} size="md" /> : (
        <span
          className={cn(
            'flex h-5 w-5 shrink-0 items-center justify-center rounded border transition-colors',
            checked
              ? 'border-[var(--sandbox-console-brand)] bg-[var(--sandbox-console-brand)] text-white'
              : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] text-transparent',
          )}
        >
          <span className="i-ph:check-bold text-xs" />
        </span>
      )}
      <span className="min-w-0">
        <span className="block font-display text-base font-bold tracking-tight text-[var(--sandbox-console-text)]">{label}</span>
        {detail ? <span className="mt-0.5 block text-sm leading-6 text-[var(--sandbox-console-muted)] group-hover:text-[var(--sandbox-console-secondary)]">{detail}</span> : null}
      </span>
    </button>
  );
}

function ResourceSizingControls({
  job,
  values,
  onChange,
}: {
  job: JobDefinition;
  values: Record<string, unknown>;
  onChange: (name: string, value: unknown) => void;
}) {
  return (
    <div className="space-y-2">
      <p className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">Resources</p>
      <div className="grid grid-cols-3 gap-2">
        <ResourceNumberInput
          label="CPU Cores"
          shortLabel="CPU"
          unit="cores"
          identity={getResourceIdentity('cpu')}
          field={field(job, 'cpuCores')}
          value={valueNumber(values, 'cpuCores', 2)}
          onChange={(value) => onChange('cpuCores', value)}
        />
        <ResourceNumberInput
          label="Memory (MB)"
          shortLabel="RAM"
          unit="MB"
          identity={getResourceIdentity('memory')}
          field={field(job, 'memoryMb')}
          value={valueNumber(values, 'memoryMb', 2048)}
          onChange={(value) => onChange('memoryMb', value)}
        />
        <ResourceNumberInput
          label="Disk (GB)"
          shortLabel="Disk"
          unit="GB"
          identity={getResourceIdentity('disk')}
          field={field(job, 'diskGb')}
          value={valueNumber(values, 'diskGb', 10)}
          onChange={(value) => onChange('diskGb', value)}
        />
      </div>
    </div>
  );
}

function ResourceNumberInput({
  label,
  shortLabel,
  unit,
  identity,
  field: fieldDef,
  value,
  onChange,
}: {
  label: string;
  shortLabel: string;
  unit: string;
  identity: IdentityMeta;
  field?: JobFieldDef;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="group block min-w-0 cursor-text rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] p-3 shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,transform] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] focus-within:border-[var(--sandbox-console-brand-border)] focus-within:bg-[var(--sandbox-console-control-hover)] focus-within:shadow-[var(--sandbox-console-control-shadow-focus)]">
      <span className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-1.5">
          <IdentityMark identity={identity} size="sm" className="h-5 w-5 rounded-[4px] text-[9px]" />
          <span className="whitespace-nowrap font-display text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--sandbox-console-muted)] group-hover:text-[var(--sandbox-console-secondary)]">{shortLabel}</span>
        </span>
        <span className="i-ph:pencil-simple-line hidden shrink-0 text-sm text-[var(--sandbox-console-brand)] opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 sm:inline-block" />
      </span>
      <span className="mt-1.5 flex min-w-0 items-baseline gap-1.5">
        <input
          aria-label={label}
          type="number"
          min={fieldDef?.min}
          max={fieldDef?.max}
          step={fieldDef?.step ?? 1}
          value={value}
          onChange={(event) => onChange(clampNumber(Number(event.target.value), fieldDef?.min, fieldDef?.max))}
          className="min-w-0 flex-1 bg-transparent font-data text-xl font-bold leading-none text-[var(--sandbox-console-text)] outline-none"
        />
        <span className="shrink-0 font-data text-[11px] font-bold uppercase text-[var(--sandbox-console-subtle)]">{unit}</span>
      </span>
    </label>
  );
}

function LaunchSpecComposer({
  blueprint,
  job,
  values,
  errors,
  entityLabel,
  nameInputRef,
  attemptedContinue,
  runtimeBackend,
  selectedImage,
  supportsAgentConfiguration,
  usesBundledAgentSelector,
  configuredAgentIdentifier,
  allHarnessEnabled,
  computerUseEnabled,
  portsInput,
  isTeeBlueprint,
  onChange,
  onPortsChange,
  onAdvancedOpen,
}: {
  blueprint?: BlueprintDefinition;
  job: JobDefinition;
  values: Record<string, unknown>;
  errors: Record<string, string | undefined>;
  entityLabel: string;
  nameInputRef: RefObject<HTMLInputElement>;
  attemptedContinue: boolean;
  runtimeBackend: string;
  selectedImage: string;
  supportsAgentConfiguration: boolean;
  usesBundledAgentSelector: boolean;
  configuredAgentIdentifier: string;
  allHarnessEnabled: boolean;
  computerUseEnabled: boolean;
  portsInput: string;
  isTeeBlueprint: boolean;
  onChange: (name: string, value: unknown) => void;
  onPortsChange: (value: string) => void;
  onAdvancedOpen: () => void;
}) {
  const imageOptions = fieldOptions(job, 'image');
  const nameError = attemptedContinue && !String(values.name || '').trim()
    ? `${entityLabel} name is required`
    : errors.name;

  return (
    <ConsoleSection title={`${entityLabel} Spec`}>
      <div className="space-y-5 p-5">
        <div className="flex flex-wrap items-start justify-between gap-4 border-b border-[var(--sandbox-console-border)] pb-5">
          <div className="flex min-w-0 items-start gap-4">
            <IdentityMark identity={getBlueprintIdentity(blueprint?.id)} size="lg" />
            <div className="min-w-0">
              <h2 className="truncate font-display text-2xl font-bold tracking-tight text-[var(--sandbox-console-text)]">
                {blueprint?.name ?? entityLabel}
              </h2>
              <p className="mt-1 max-w-2xl text-[15px] leading-6 text-[var(--sandbox-console-muted)]">
                {blueprint?.description}
              </p>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <ConsoleChip tone={runtimeBackend === 'tee' ? 'warn' : 'ready'}>{runtimeLabel(runtimeBackend)}</ConsoleChip>
            {isTeeBlueprint ? <ConsoleChip tone="warn">TEE path</ConsoleChip> : null}
          </div>
        </div>

        <div className="grid gap-5 lg:grid-cols-[minmax(0,1.05fr)_minmax(280px,0.95fr)]">
          <div className="space-y-4">
            <LaunchInput
              label={`${entityLabel} Name`}
              inputRef={nameInputRef}
              value={valueString(values, 'name')}
              onChange={(event) => onChange('name', event.target.value)}
              placeholder={field(job, 'name')?.placeholder ?? 'agent-workspace'}
              error={nameError}
            />

            <LaunchImageSelect
              value={selectedImage}
              options={imageOptions}
              onChange={(value) => onChange('image', value)}
              placeholder={field(job, 'image')?.placeholder ?? 'ghcr.io/tangle-network/blueprint-sidecar:all-harness'}
            />
          </div>

          <div className="space-y-4">
            <SegmentedControl
              label="Runtime Backend"
              value={runtimeBackend}
              options={fieldOptions(job, 'runtimeBackend')}
              onChange={(value) => onChange('runtimeBackend', value)}
            />
            <LaunchNativeSelect
              label="Stack"
              value={valueString(values, 'stack', 'default')}
              options={fieldOptions(job, 'stack').map((option) => ({
                ...option,
                detail: getStackIdentity(option.value).detail,
                identity: getStackIdentity(option.value),
              }))}
              onChange={(value) => onChange('stack', value)}
            />
            <ResourceSizingControls job={job} values={values} onChange={onChange} />
          </div>
        </div>

        {supportsAgentConfiguration ? (
          <AgentConfigurationField
            image={selectedImage}
            value={configuredAgentIdentifier}
            usesBundledSelector={usesBundledAgentSelector}
            onChange={(next) => onChange('agentIdentifier', next)}
          />
        ) : null}

        <div className="grid gap-3 lg:grid-cols-2">
          <AllHarnessCapabilityField
            enabled={allHarnessEnabled}
            onChange={(enabled) => onChange('capabilitiesJson', setCapabilityJson(values.capabilitiesJson, 'all_harness', enabled))}
          />
          <ComputerUseCapabilityField
            enabled={computerUseEnabled}
            onChange={(enabled) => onChange('capabilitiesJson', setCapabilityJson(values.capabilitiesJson, 'computer_use', enabled))}
          />
        </div>

        <div className="grid gap-3 lg:grid-cols-2">
          <LaunchToggle
            label="Enable SSH"
            checked={Boolean(values.sshEnabled)}
            onChange={(enabled) => onChange('sshEnabled', enabled)}
            identity={getCapabilityIdentity('ssh')}
            detail="Expose an operator-managed SSH entrypoint after provisioning."
          />
        </div>

        {Boolean(values.sshEnabled) ? (
          <LaunchTextArea
            label="SSH Public Key"
            value={valueString(values, 'sshPublicKey')}
            onChange={(event) => onChange('sshPublicKey', event.target.value)}
            placeholder={field(job, 'sshPublicKey')?.placeholder ?? 'ssh-ed25519 AAAA...'}
          />
        ) : null}

        {configuredAgentIdentifier ? (
          <div className="rounded-[5px] border border-amber-400/25 bg-amber-400/10 px-3.5 py-2.5">
            <p className="text-sm leading-6 text-amber-200">
              This agent needs AI credentials to chat. Add them as environment variables now or inject them later through Secrets.
            </p>
          </div>
        ) : null}

        <div className="grid gap-5 border-t border-[var(--sandbox-console-border)] pt-5 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.7fr)]">
          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <p className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">Environment Variables</p>
              <span className="font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">injected at boot</span>
            </div>
            <EnvEditor
              value={String(values.envJson || '{}')}
              onChange={(json) => onChange('envJson', json)}
            />
          </div>

          <div className="space-y-4">
            <LaunchInput
              label="Exposed Ports"
              value={portsInput}
              onChange={(event) => onPortsChange(event.target.value)}
              placeholder="3000, 8080, 5432"
              detail={runtimeBackend === 'firecracker' ? 'Firecracker DNAT' : 'operator proxy'}
            />
            <LaunchActionButton variant="secondary" className="w-full" onClick={onAdvancedOpen}>
              <span className="i-ph:sliders-horizontal text-base" />
              Advanced Settings
            </LaunchActionButton>
          </div>
        </div>
      </div>
    </ConsoleSection>
  );
}

function LaunchSummaryPanel({
  step,
  selectedBlueprint,
  entityLabel,
  runtimeBackend,
  infra,
  capacity,
  isConnected,
  isReconnectingWallet,
  hasValidService,
  isNewService,
  serviceValidating,
  serviceError,
  operatorsCount,
  operatorCount,
  operatorsLoading,
  operatorsError,
  agentIdentifier,
  ports,
  onOpenInfra,
}: {
  step: WizardStep;
  selectedBlueprint?: BlueprintDefinition;
  entityLabel: string;
  runtimeBackend: string;
  infra: { blueprintId: string; serviceId: string };
  capacity?: number | bigint;
  isConnected: boolean;
  isReconnectingWallet: boolean;
  hasValidService: boolean;
  isNewService: boolean;
  serviceValidating: boolean;
  serviceError: string | null;
  operatorsCount: number;
  operatorCount: bigint;
  operatorsLoading: boolean;
  operatorsError?: Error | null;
  agentIdentifier: string;
  ports: number[];
  onOpenInfra: () => void;
}) {
  const operatorSummary = operatorsLoading
    ? 'Discovering'
    : operatorsError && operatorCount > 0n
      ? `${operatorCount.toString()} registered`
      : operatorsError
        ? 'Lookup failed'
        : `${operatorsCount} verified`;
  const serviceState = serviceValidating
    ? 'Checking'
    : serviceError
      ? 'Blocked'
      : isNewService
        ? 'Create on deploy'
        : hasValidService
          ? 'Verified'
          : 'Pending';

  return (
    <aside className="space-y-4">
      <ConsoleSection title="Deploy Summary" className="xl:sticky xl:top-0">
        <div className="divide-y divide-[var(--sandbox-console-border)]">
          <SummaryRow
            label="Mode"
            value={selectedBlueprint ? entityLabel : 'Unselected'}
            detail={selectedBlueprint?.name ?? 'Choose a blueprint'}
            identity={getBlueprintIdentity(selectedBlueprint?.id)}
            tone={selectedBlueprint ? 'brand' : 'warn'}
          />
          <SummaryRow
            label="Spec"
            value={step === 'deploy' ? 'Locked' : step === 'configure' ? 'Editing' : 'Open'}
            detail={step === 'deploy' ? 'ready for transaction' : 'mutable'}
            identity={step === 'deploy'
              ? { label: 'Locked', mark: 'OK', detail: 'ready for transaction', icon: 'i-ph:lock-key', tone: 'teal' }
              : { label: 'Editing', mark: 'ED', detail: 'mutable config', icon: 'i-ph:pencil-simple-line', tone: 'slate' }}
            tone={step === 'deploy' ? 'ready' : 'muted'}
          />
          <SummaryRow
            label="Runtime"
            value={runtimeLabel(runtimeBackend)}
            detail={runtimeBackend === 'tee' ? 'attestation path' : 'standard path'}
            identity={getRuntimeIdentity(runtimeBackend)}
            tone={runtimeBackend === 'tee' ? 'warn' : 'ready'}
          />
          <SummaryRow
            label="Capacity"
            value={formatCapacityValue(capacity)}
            detail="available slots"
            identity={{ label: 'Capacity', mark: 'CAP', detail: 'available slots', icon: 'i-ph:database', tone: 'blue' }}
            tone={capacity !== undefined && Number(capacity) === 0 ? 'warn' : 'ready'}
          />
          <SummaryRow
            label="Wallet"
            value={isConnected ? 'Connected' : isReconnectingWallet ? 'Syncing' : 'Offline'}
            detail={isConnected ? 'can sign' : 'deploy blocked'}
            identity={isConnected
              ? { label: 'Wallet', mark: 'WAL', detail: 'connected signer', icon: 'i-ph:wallet', tone: 'teal' }
              : { label: 'Wallet', mark: 'WAL', detail: 'deploy blocked', icon: 'i-ph:wallet', tone: 'danger' }}
            tone={isConnected ? 'ready' : isReconnectingWallet ? 'warn' : 'danger'}
          />
          <SummaryRow
            label="Service"
            value={serviceState}
            detail={`bp ${infra.blueprintId || '--'} / svc ${infra.serviceId || '--'}`}
            identity={{ label: 'Service', mark: 'SVC', detail: 'on-chain service', icon: 'i-ph:tree-structure', tone: serviceState === 'Blocked' ? 'danger' : 'brand' }}
            tone={serviceTone({ serviceValidating, serviceError, hasValidService, isNewService })}
          />
          <SummaryRow
            label="Operators"
            value={operatorSummary}
            detail={isNewService ? 'service quorum' : 'operator service'}
            identity={getOperatorIdentity()}
            tone={operatorsError ? 'warn' : 'brand'}
          />
          <SummaryRow
            label="Agent mode"
            value={agentIdentifier || 'Compute only'}
            detail={agentIdentifier ? 'chat enabled' : 'no bundled agent'}
            identity={getAgentIdentity(agentIdentifier)}
            tone={agentIdentifier ? 'brand' : 'muted'}
          />
          <SummaryRow
            label="Network"
            value={ports.length > 0 ? `${ports.length} port${ports.length === 1 ? '' : 's'}` : 'Default'}
            detail={ports.length > 0 ? ports.join(', ') : 'operator proxy'}
            identity={getResourceIdentity('network')}
            tone={ports.length > 0 ? 'brand' : 'muted'}
          />
        </div>
        <div className="border-t border-[var(--sandbox-console-border)] p-3">
          <LaunchActionButton variant="secondary" size="sm" className="w-full" onClick={onOpenInfra}>
            <span className="i-ph:sliders-horizontal text-sm" />
            Infrastructure
          </LaunchActionButton>
        </div>
      </ConsoleSection>
    </aside>
  );
}

function SummaryRow({
  label,
  value,
  detail,
  identity,
  tone,
}: {
  label: string;
  value: string;
  detail: string;
  identity?: IdentityMeta;
  tone: ConsoleTone;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-4 py-4 transition-colors hover:bg-[var(--sandbox-console-hover)]">
      <span className="flex min-w-0 items-center gap-2.5">
        {identity ? <IdentityMark identity={identity} size="sm" /> : null}
        <span className="min-w-0">
          <span className="block font-data text-[11px] font-bold uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
            {label}
          </span>
          <span className="mt-1 block truncate font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">
            {detail}
          </span>
        </span>
      </span>
      <span className={cn('max-w-52 text-right font-data text-xl font-bold leading-tight tracking-tight', executionMetricToneClass[tone])}>
        {value}
      </span>
    </div>
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
    ? 'Choose an agent already bundled in this image. None keeps the resource compute-only and hides chat.'
    : 'Custom images must already register this agent identifier internally. Typing a new name here does not create a new agent.';
  const selectValue = value || BUNDLED_NO_AGENT_VALUE;

  return (
    <div className="border-t border-[var(--sandbox-console-border)] pt-4">
      {usesBundledSelector ? (
        <LaunchNativeSelect
          label="Agent"
          value={selectValue}
          onChange={(next) => onChange(sanitizeBundledAgentIdentifier(next))}
          options={BUNDLED_AGENT_OPTIONS.map((option) => ({
            ...option,
            detail: getAgentIdentity(option.value).detail,
            identity: getAgentIdentity(option.value),
          }))}
        />
      ) : (
        <LaunchInput
          label="Agent"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={image ? 'default' : 'Choose an image first'}
        />
      )}
      <p className="mt-2 text-sm leading-6 text-[var(--sandbox-console-muted)]">
        {helpText}
      </p>
      {!usesBundledSelector && value.trim() !== '' && (
        <div className="mt-3 rounded-[5px] border border-amber-400/25 bg-amber-400/10 px-3.5 py-2.5">
          <p className="text-sm leading-6 text-amber-200">
            Custom agent identifiers depend on the selected image registering the agent
            internally. If the image does not recognize this name, chat will fail after provision.
          </p>
        </div>
      )}
    </div>
  );
}

function AllHarnessCapabilityField({
  enabled,
  onChange,
}: {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <LaunchToggle
      label="All-Harness Runtime"
      checked={enabled}
      onChange={onChange}
      identity={getCapabilityIdentity('harness')}
      detail="Request Claude, Codex, opencode, Kimi, and Gemini harnesses in the sidecar image."
    />
  );
}

function ComputerUseCapabilityField({
  enabled,
  onChange,
}: {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <LaunchToggle
      label="Computer Use"
      checked={enabled}
      onChange={onChange}
      identity={getCapabilityIdentity('computer-use')}
      detail="Enable browser/computer-use tools for visual agent tasks when the sidecar image supports them."
    />
  );
}

function AdvancedOptionsModal({
  open,
  onOpenChange,
  job,
  values,
  runtimeBackend,
  isTeeBlueprint,
  onChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  job: JobDefinition;
  values: Record<string, unknown>;
  runtimeBackend: string;
  isTeeBlueprint: boolean;
  onChange: (name: string, value: unknown) => void;
}) {
  useEffect(() => {
    if (!open) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onOpenChange(false);
    }
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [open, onOpenChange]);

  if (!open) return null;

  const maxLifetimeHours = hoursFromSeconds(values.maxLifetimeSeconds, 86400);
  const idleTimeoutMinutes = minutesFromSeconds(values.idleTimeoutSeconds, 3600);
  const showTeeControls = isTeeBlueprint || runtimeBackend === 'tee';
  const teeRequiredLocked = runtimeBackend === 'tee';

  return (
    <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm" role="presentation" onMouseDown={() => onOpenChange(false)}>
      <div
        className="w-full max-w-2xl overflow-hidden rounded-none border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] shadow-[var(--sandbox-console-shadow-lg)]"
        role="dialog"
        aria-modal="true"
        aria-label="Advanced Settings"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3 border-b border-[var(--sandbox-console-border)] px-4 py-3">
          <div>
            <h2 className="font-display text-base font-semibold text-[var(--sandbox-console-text)]">Advanced Settings</h2>
            <p className="mt-0.5 text-xs text-[var(--sandbox-console-muted)]">Runtime limits, metadata, and confidential compute flags.</p>
          </div>
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="flex h-8 w-8 items-center justify-center rounded-[4px] text-[var(--sandbox-console-muted)] transition-colors hover:bg-[var(--sandbox-console-hover)] hover:text-[var(--sandbox-console-text)]"
            aria-label="Close advanced settings"
          >
            <span className="i-ph:x text-base" />
          </button>
        </div>

        <div className="max-h-[70vh] space-y-4 overflow-y-auto p-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <LaunchInput
              label="Max Lifetime"
              detail="hours"
              type="number"
              min={0}
              step={1}
              value={maxLifetimeHours}
              onChange={(event) => onChange('maxLifetimeSeconds', Math.max(0, Number(event.target.value)) * 3600)}
            />
            <LaunchInput
              label="Idle Timeout"
              detail="minutes"
              type="number"
              min={0}
              step={5}
              value={idleTimeoutMinutes}
              onChange={(event) => onChange('idleTimeoutSeconds', Math.max(0, Number(event.target.value)) * 60)}
            />
          </div>

          {showTeeControls ? (
            <div className="grid gap-3 sm:grid-cols-2">
              <LaunchToggle
                label="TEE Required"
                checked={Boolean(values.teeRequired) || teeRequiredLocked}
                disabled={teeRequiredLocked}
                onChange={(enabled) => onChange('teeRequired', enabled)}
                identity={getRuntimeIdentity('tee')}
                detail={teeRequiredLocked ? 'Pinned by TEE runtime' : 'Require attested hardware isolation.'}
              />
              <LaunchNativeSelect
                label="TEE Type"
                value={valueString(values, 'teeType', '0')}
                options={fieldOptions(job, 'teeType').map((option) => ({
                  ...option,
                  identity: option.value === '0'
                    ? { label: option.label, mark: 'OFF', detail: 'not required', icon: 'i-ph:minus-circle', tone: 'slate' }
                    : { label: option.label, mark: option.label.slice(0, 3).toUpperCase(), detail: 'attestation target', icon: 'i-ph:shield-check', tone: 'amber' },
                }))}
                onChange={(value) => onChange('teeType', value)}
              />
            </div>
          ) : null}

          <LaunchTextArea
            label="Metadata JSON"
            value={valueString(values, 'metadataJson', '{}')}
            onChange={(event) => onChange('metadataJson', event.target.value)}
            placeholder="{}"
            className="min-h-40 font-data"
          />
        </div>

        <div className="flex justify-end gap-2 border-t border-[var(--sandbox-console-border)] p-3">
          <LaunchActionButton variant="secondary" onClick={() => onOpenChange(false)}>Done</LaunchActionButton>
        </div>
      </div>
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
  onCreateService: () => void;
  onOpenInfra: () => void;
  onOpenOperators: () => void;
  onViewDetail: () => void;
  onProvisionReady: (sandboxId: string, sidecarUrl: string) => void;
}

type DeployBlocker = {
  title: string;
  detail: string;
  icon: string;
  tone: ConsoleTone;
};

const preflightToneClass: Record<ConsoleTone, string> = {
  brand: 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-brand)]',
  ready: 'bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)]',
  warn: 'bg-amber-400/10 text-amber-300',
  danger: 'bg-red-400/10 text-[var(--sandbox-console-danger)]',
  muted: 'bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)]',
};

const preflightPanelClass: Record<ConsoleTone, string> = {
  brand: 'bg-[var(--sandbox-console-brand-soft)] ring-[var(--sandbox-console-brand-border)]',
  ready: 'bg-[var(--sandbox-console-success-soft)] ring-[var(--sandbox-console-success-border)]',
  warn: 'bg-amber-400/[0.08] ring-amber-400/25',
  danger: 'bg-red-400/[0.08] ring-red-400/25',
  muted: 'bg-[var(--sandbox-console-control)] ring-[var(--sandbox-console-border)]',
};

function getServiceProblem({
  serviceInfo,
  serviceError,
  serviceId,
  blueprintId,
  address,
}: {
  serviceInfo: { active: boolean; permitted: boolean } | null;
  serviceError: string | null;
  serviceId: string;
  blueprintId: string;
  address?: string;
}): DeployBlocker | null {
  const formattedService = serviceId || '--';
  const formattedBlueprint = blueprintId || '--';

  if (serviceError) {
    return {
      title: `Service #${formattedService} not found`,
      detail: `Create a service for blueprint #${formattedBlueprint}, or choose an active service before deploying this sandbox.`,
      icon: 'i-ph:x-circle',
      tone: 'danger',
    };
  }

  if (serviceInfo && !serviceInfo.active) {
    return {
      title: `Service #${formattedService} is inactive`,
      detail: 'Choose an active service or create a replacement service before deploying this sandbox.',
      icon: 'i-ph:power',
      tone: 'warn',
    };
  }

  if (serviceInfo && !serviceInfo.permitted) {
    return {
      title: `Wallet not permitted on service #${formattedService}`,
      detail: `${address ? truncateAddress(address) : 'This wallet'} cannot submit jobs to this service. Choose or create a service where this wallet is allowed.`,
      icon: 'i-ph:lock-key',
      tone: 'danger',
    };
  }

  return null;
}

function getDeployBlocker({
  status,
  serviceValidating,
  serviceProblem,
  contractsDeployed,
  isConnected,
  isReconnecting,
  isSandbox,
  capacity,
  isNewService,
  operatorsLoading,
  operatorCount,
  priceLoading,
}: {
  status: DeployStatus;
  serviceValidating: boolean;
  serviceProblem: DeployBlocker | null;
  contractsDeployed: boolean;
  isConnected: boolean;
  isReconnecting: boolean;
  isSandbox: boolean;
  capacity?: number | bigint;
  isNewService: boolean;
  operatorsLoading: boolean;
  operatorCount: number;
  priceLoading: boolean;
}): DeployBlocker | null {
  if (status !== 'idle') return null;
  if (serviceValidating) {
    return {
      title: 'Checking service',
      detail: 'Reading the selected service before the transaction can be built.',
      icon: 'i-ph:spinner',
      tone: 'warn',
    };
  }
  if (!contractsDeployed) {
    return {
      title: 'Contracts unavailable',
      detail: 'Switch to a supported network before deploying.',
      icon: 'i-ph:warning-circle',
      tone: 'danger',
    };
  }
  if (isReconnecting) {
    return {
      title: 'Wallet reconnecting',
      detail: 'Wait for the wallet session to finish reconnecting.',
      icon: 'i-ph:wallet',
      tone: 'warn',
    };
  }
  if (!isConnected) {
    return {
      title: 'Connect wallet',
      detail: 'A connected wallet is required to create services and submit jobs.',
      icon: 'i-ph:wallet',
      tone: 'danger',
    };
  }
  if (isSandbox && capacity !== undefined && Number(capacity) === 0) {
    return {
      title: 'No sandbox capacity',
      detail: 'All operator slots are in use. Delete unused sandboxes or try again later.',
      icon: 'i-ph:database',
      tone: 'warn',
    };
  }
  if (serviceProblem) return serviceProblem;
  if (isNewService && operatorsLoading) {
    return {
      title: 'Finding operators',
      detail: 'Operator discovery must finish before a new service request can be created.',
      icon: 'i-ph:users-three',
      tone: 'warn',
    };
  }
  if (isNewService && operatorCount === 0) {
    return {
      title: 'No operators available',
      detail: 'This blueprint needs at least one registered operator before a service can be created.',
      icon: 'i-ph:users-three',
      tone: 'danger',
    };
  }
  if (priceLoading) {
    return {
      title: 'Loading price',
      detail: 'Waiting for the operator quote before showing the deploy transaction.',
      icon: 'i-ph:receipt',
      tone: 'warn',
    };
  }

  return null;
}

function DeploySpecPill({
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

function DeployStep({
  blueprint, job, values, ports, infra, entityLabel, deploy,
  capacity, provisionEstimate, provisionPriceFormatted,
  hasProvisionRfq, priceLoading,
  serviceInfo, serviceValidating, serviceError,
  onBack, onDeploy, onCreateService, onOpenInfra, onOpenOperators, onViewDetail, onProvisionReady,
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
        'overflow-hidden rounded-[4px] bg-[var(--sandbox-console-surface)] shadow-[0_18px_44px_rgba(0,0,0,0.14)] ring-1',
        preflightTone === 'danger'
          ? 'ring-red-400/30'
          : preflightTone === 'warn'
            ? 'ring-amber-400/30'
            : 'ring-[var(--sandbox-console-border)]',
      )}>
        <div className="grid gap-px bg-[var(--sandbox-console-border)] xl:grid-cols-[minmax(0,1fr)_minmax(300px,380px)]">
          <div className="bg-[var(--sandbox-console-panel)] p-4 sm:p-5">
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

          <div className="flex flex-col justify-between bg-[var(--sandbox-console-panel-strong)] p-4 sm:p-5">
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
                <>
                  <LaunchActionButton size="lg" className="w-full" onClick={onCreateService}>
                    <span className="i-ph:plus-circle text-base" />
                    Create service
                  </LaunchActionButton>
                  <div className="grid grid-cols-2 gap-2">
                    <LaunchActionButton variant="secondary" size="sm" onClick={onOpenInfra}>
                      <span className="i-ph:magnifying-glass text-sm" />
                      Choose service
                    </LaunchActionButton>
                    <LaunchActionButton variant="secondary" size="sm" onClick={onOpenOperators}>
                      <span className="i-ph:users-three text-sm" />
                      Operators
                    </LaunchActionButton>
                  </div>
                </>
              ) : (
                <DeployButton
                  status={status}
                  canDeploy={deploy.canDeploy}
                  isNewService={isNewService}
                  priceLoading={priceLoading}
                  serviceValidating={serviceValidating}
                  costDisplay={costDisplay}
                  blockedTitle={deployBlocker?.title}
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

      {/* ── Operators (new service or missing cloud service, idle) ── */}
      {(isNewService || serviceNeedsSetup) && status === 'idle' && (
        <OperatorList
          operators={operators}
          operatorsLoading={operatorsLoading}
          operatorsError={operatorsError}
          operatorCount={operatorCount}
          blueprintId={infra.blueprintId}
          purpose={serviceNeedsSetup ? 'service' : 'instance'}
        />
      )}

    </div>
  );
}

// ── Sub-components (extracted for readability) ──

const executionMetricToneClass: Record<ConsoleTone, string> = {
  brand: 'text-[var(--sandbox-console-brand)]',
  ready: 'text-[var(--sandbox-console-success)]',
  warn: 'text-[var(--sandbox-console-warning)]',
  danger: 'text-[var(--sandbox-console-danger)]',
  muted: 'text-[var(--sandbox-console-text)]',
};

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
    : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)]';

  const messages: Record<DeployStatus, string> = {
    idle: '',
    signing: isNewService ? 'Confirm service creation in wallet...' : 'Confirm in wallet...',
    pending: isNewService ? 'Creating service on-chain...' : 'Confirming on-chain...',
    confirmed: isNewService ? 'Service created — waiting for operator provisioning' : `${entityLabel} creation confirmed`,
    provisioning: 'Operator provisioning in progress...',
    ready: `${entityLabel} is ready`,
    failed: 'Transaction failed',
  };

  const icons: Record<DeployStatus, ReactNode> = {
    idle: null,
    signing: <div className="w-5 h-5 rounded-full border-2 border-amber-400 border-t-transparent animate-spin" />,
    pending: <div className="w-5 h-5 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />,
    confirmed: <div className="i-ph:check-circle-fill text-lg text-teal-400" />,
    provisioning: <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />,
    ready: <div className="i-ph:check-circle-fill text-lg text-teal-400" />,
    failed: <div className="i-ph:x-circle-fill text-lg text-crimson-400" />,
  };

  return (
    <div className={cn('rounded-[5px] border p-4', borderClass)}>
      <div className="flex items-center gap-3">
        {icons[status]}
        <div className="flex-1 min-w-0">
          <p className="font-display text-base font-bold text-[var(--sandbox-console-text)]">
            {messages[status]}
          </p>
          {txHash && (
            <p className="mt-1 truncate font-data text-xs text-[var(--sandbox-console-muted)]">{txHash}</p>
          )}
          {error && (
            <div className="mt-1">
              <p className="text-xs text-crimson-400">{error}</p>
              {/resource not available|request already pending/i.test(error) && (
                <p className="mt-1 text-[11px] text-[var(--sandbox-console-muted)]">
                  MetaMask may have a pending request. Open MetaMask, dismiss any popups, then try again.
                </p>
              )}
              {/user rejected|denied/i.test(error) && (
                <p className="mt-1 text-[11px] text-[var(--sandbox-console-muted)]">
                  Transaction was rejected in the wallet.
                </p>
              )}
              {/chain.*mismatch|wrong.*network/i.test(error) && (
                <p className="mt-1 text-[11px] text-[var(--sandbox-console-muted)]">
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
      'rounded-[5px] border p-4',
      provision ? 'border-teal-500/20 bg-teal-500/[0.03]' : 'border-violet-500/20 bg-violet-500/[0.03]',
    )}>
      <div className="flex items-center gap-3">
        {provision ? (
          <>
            <div className="i-ph:check-circle-fill text-lg text-teal-400" />
            <div>
              <p className="font-display text-base font-bold text-teal-400">Instance ready</p>
              <p className="mt-1 max-w-sm truncate font-data text-xs text-[var(--sandbox-console-muted)]">
                {provision.sidecarUrl}
              </p>
            </div>
          </>
        ) : (
          <>
            <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />
            <div>
              <p className="font-display text-base font-bold text-[var(--sandbox-console-text)]">Waiting for operator...</p>
              <p className="mt-1 text-xs text-[var(--sandbox-console-muted)]">Watching for on-chain provisioning event</p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function OperatorList({
  operators,
  operatorsLoading,
  operatorsError,
  operatorCount,
  blueprintId,
  purpose = 'instance',
}: {
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  operatorsError?: Error | null;
  operatorCount: bigint;
  blueprintId: string;
  purpose?: 'instance' | 'service';
}) {
  const titleCount = operatorsLoading
    ? '...'
    : operatorsError && operatorCount > 0n
      ? operatorCount.toString()
      : String(operators.length);

  return (
    <div className="sandbox-console-panel rounded-[5px] p-4">
      <div className="flex items-center gap-2 mb-3">
        <div className="i-ph:users-three text-base text-[var(--sandbox-console-muted)]" />
        <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">
          Operators ({titleCount})
        </span>
      </div>
      {operatorsLoading ? (
        <div className="flex items-center gap-2">
          <div className="h-3 w-3 animate-spin rounded-full border border-[var(--sandbox-console-muted)] border-t-transparent" />
          <span className="text-xs text-[var(--sandbox-console-muted)]">Discovering operators for blueprint #{blueprintId}...</span>
        </div>
      ) : operatorsError ? (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <div className="i-ph:warning-circle text-sm text-amber-400" />
            <span className="text-xs text-amber-400">
              {operatorCount > 0n
                ? `Found ${operatorCount.toString()} registered operator${operatorCount === 1n ? '' : 's'} on-chain, but verification failed`
                : 'Operator lookup failed for this blueprint'}
            </span>
          </div>
          <p className="text-sm leading-6 text-[var(--sandbox-console-muted)]">
            This is usually a local RPC or multicall issue. The app could not build a verified operator list for service creation.
          </p>
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
              <OperatorIdentity address={op.address} detail="registered operator" />
            </div>
          ))}
          <p className="mt-2 text-sm leading-6 text-[var(--sandbox-console-muted)]">
            {purpose === 'service'
              ? 'Use Create Service to request an active cloud service with these registered operators, then deploy the sandbox into that service.'
              : 'A new service will be created with these operators. Your sandbox config will be passed as service request inputs.'}
          </p>
        </div>
      )}
    </div>
  );
}

function DeployButton({
  status, canDeploy, isNewService, priceLoading, serviceValidating, costDisplay, blockedTitle, onDeploy,
}: {
  status: DeployStatus;
  canDeploy: boolean;
  isNewService: boolean;
  priceLoading: boolean;
  serviceValidating: boolean;
  costDisplay: string;
  blockedTitle?: string;
  onDeploy: () => void;
}) {
  const isBusy = status === 'signing' || status === 'pending';
  const isDisabled = !canDeploy || isBusy || priceLoading || serviceValidating;

  return (
    <LaunchActionButton size="lg" className="w-full" onClick={onDeploy} disabled={isDisabled}>
      {isBusy ? (
        <>
          <div className="w-4 h-4 rounded-full border-2 border-white/40 border-t-white animate-spin" />
          {status === 'signing' ? 'Confirm in wallet...' : 'Deploying...'}
        </>
      ) : priceLoading ? (
        'Loading price...'
      ) : blockedTitle ? (
        <>
          <div className="i-ph:lock-key text-base" />
          {blockedTitle}
        </>
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
    </LaunchActionButton>
  );
}
