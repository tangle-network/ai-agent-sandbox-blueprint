import { useState, useCallback, useMemo, useEffect, useRef, type ButtonHTMLAttributes, type InputHTMLAttributes, type RefObject, type ReactNode, type TextareaHTMLAttributes } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { InfrastructureModal } from '~/components/shared/InfrastructureModal';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { infraStore, updateInfra } from '@tangle-network/blueprint-ui';
import { Identicon } from '@tangle-network/blueprint-ui/components';
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
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [attemptedContinue, setAttemptedContinue] = useState(false);
  const nameInputRef = useRef<HTMLInputElement>(null);

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

  const showConnectPanel = !isConnected && !address && !isReconnectingWallet;
  const parsedPorts = parsePortsInput(portsInput);

  return (
    <ConsolePage
      title="Launch Workspace"
      eyebrow="Tangle agent compute"
      actions={step !== 'blueprint' ? (
        <LaunchActionButton variant="secondary" onClick={() => setShowInfra(true)}>
          <span className="i-ph:sliders-horizontal text-base" />
          Infrastructure
        </LaunchActionButton>
      ) : null}
    >
      <div className="grid min-h-full gap-4 xl:grid-cols-[minmax(0,1fr)_340px]">
        <main className="min-w-0 space-y-4">
          {showConnectPanel && (
            <ConnectWalletPanel
              description="Provisioning a sandbox or instance requires a connected wallet on Tangle Network. You can browse blueprints below, but deploying will be blocked until you connect."
            />
          )}

          <LaunchModeStrip
            blueprints={blueprints}
            selectedBlueprint={selectedBlueprint}
            onSelect={handleSelectBlueprint}
          />

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
            <div className="space-y-4">
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
              <div className="flex justify-between">
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
              onViewDetail={() => {
                const key = isSandbox
                  ? deploy.sandboxDraftKey
                  : String(values.name || '');
                if (key) navigate(`/${isSandbox ? 'sandboxes' : 'instances'}/${encodeURIComponent(key)}`);
                else navigate(isSandbox ? '/sandboxes' : '/instances');
              }}
              onOpenInfra={() => setShowInfra(true)}
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
          onOpenInfra={() => setShowInfra(true)}
        />
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

      <InfrastructureModal open={showInfra} onOpenChange={setShowInfra} />
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
        'inline-flex items-center justify-center gap-2 rounded-md border font-display font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-50',
        size === 'sm' && 'h-8 px-3 text-xs',
        size === 'md' && 'h-10 px-4 text-sm',
        size === 'lg' && 'h-11 px-5 text-sm',
        variant === 'primary' && 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] hover:border-[var(--sandbox-console-brand)] hover:bg-[rgba(142,89,255,0.22)]',
        variant === 'secondary' && 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] text-[var(--sandbox-console-secondary)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-panel-strong)] hover:text-[var(--sandbox-console-text)]',
        variant === 'success' && 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)] hover:bg-[rgba(56,178,172,0.18)]',
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

          return (
            <button
              key={bp.id}
              type="button"
              onClick={() => onSelect(bp)}
              className={cn(
                'min-h-28 bg-[var(--sandbox-console-panel)] p-4 text-left transition-colors hover:bg-[var(--sandbox-console-hover)]',
                active && 'bg-[var(--sandbox-console-brand-soft)]',
              )}
            >
              <div className="flex items-start justify-between gap-3">
                <span className={cn('text-2xl text-[var(--sandbox-console-brand)]', bp.icon)} />
                {recommended ? <ConsoleChip tone="ready">recommended</ConsoleChip> : null}
              </div>
              <div className="mt-3">
                <p className="font-display text-sm font-semibold text-[var(--sandbox-console-text)]">{bp.name}</p>
                <p className="mt-1 line-clamp-2 text-xs leading-5 text-[var(--sandbox-console-muted)]">{bp.description}</p>
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
    <label className="block min-w-0 space-y-1.5">
      <span className="flex items-center justify-between gap-3">
        <span className="font-display text-xs font-semibold text-[var(--sandbox-console-secondary)]">{label}</span>
        {detail ? <span className="font-data text-[11px] text-[var(--sandbox-console-subtle)]">{detail}</span> : null}
      </span>
      {children}
      {error ? <span className="block text-xs text-[var(--sandbox-console-danger)]">{error}</span> : null}
    </label>
  );
}

const launchControlClass = 'w-full rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] px-3 py-2.5 font-data text-sm text-[var(--sandbox-console-text)] placeholder:text-[var(--sandbox-console-subtle)] transition-colors hover:border-[var(--sandbox-console-border-hover)] focus:border-[var(--sandbox-console-brand-border)] focus:outline-none disabled:cursor-not-allowed disabled:opacity-60';

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
  options: { label: string; value: string }[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  return (
    <LaunchField label={label} detail={detail}>
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        disabled={disabled}
        className={cn(launchControlClass, 'appearance-none bg-[var(--sandbox-console-surface)]')}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </LaunchField>
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
    <div className="space-y-1.5">
      <p className="font-display text-xs font-semibold text-[var(--sandbox-console-secondary)]">{label}</p>
      <div className="grid gap-1 rounded-md bg-[var(--sandbox-console-surface)] p-1 sm:grid-cols-3">
        {options.map((option) => {
          const active = option.value === value;
          return (
            <button
              key={option.value}
              type="button"
              onClick={() => onChange(option.value)}
              className={cn(
                'min-h-9 rounded px-3 text-center font-display text-xs font-semibold transition-colors',
                active
                  ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_0_0_0_1px_var(--sandbox-console-brand-border)]'
                  : 'text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-hover)] hover:text-[var(--sandbox-console-text)]',
              )}
            >
              {option.label.replace(' (default)', '')}
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
  checked,
  onChange,
  disabled,
}: {
  label: string;
  detail?: string;
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
        'flex w-full items-center gap-3 rounded-md border p-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-60',
        checked
          ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)]'
          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-hover)]',
      )}
    >
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
      <span className="min-w-0">
        <span className="block font-display text-sm font-semibold text-[var(--sandbox-console-text)]">{label}</span>
        {detail ? <span className="mt-0.5 block text-xs leading-5 text-[var(--sandbox-console-muted)]">{detail}</span> : null}
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
      <p className="font-display text-xs font-semibold text-[var(--sandbox-console-secondary)]">Resources</p>
      <div className="grid grid-cols-3 gap-2">
        <ResourceNumberInput
          label="CPU Cores"
          shortLabel="CPU"
          field={field(job, 'cpuCores')}
          value={valueNumber(values, 'cpuCores', 2)}
          onChange={(value) => onChange('cpuCores', value)}
        />
        <ResourceNumberInput
          label="Memory (MB)"
          shortLabel="RAM"
          field={field(job, 'memoryMb')}
          value={valueNumber(values, 'memoryMb', 2048)}
          onChange={(value) => onChange('memoryMb', value)}
        />
        <ResourceNumberInput
          label="Disk (GB)"
          shortLabel="Disk"
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
  field: fieldDef,
  value,
  onChange,
}: {
  label: string;
  shortLabel: string;
  field?: JobFieldDef;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="block min-w-0 rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] p-2 transition-colors hover:border-[var(--sandbox-console-border-hover)]">
      <span className="block truncate font-display text-[11px] font-semibold text-[var(--sandbox-console-muted)]">{shortLabel}</span>
      <input
        aria-label={label}
        type="number"
        min={fieldDef?.min}
        max={fieldDef?.max}
        step={fieldDef?.step ?? 1}
        value={value}
        onChange={(event) => onChange(clampNumber(Number(event.target.value), fieldDef?.min, fieldDef?.max))}
        className="mt-1 w-full bg-transparent font-data text-base font-semibold text-[var(--sandbox-console-text)] outline-none"
      />
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
  const imageListId = `${job.name}-image-options`;
  const nameError = attemptedContinue && !String(values.name || '').trim()
    ? `${entityLabel} name is required`
    : errors.name;

  return (
    <ConsoleSection title={`${entityLabel} Spec`}>
      <div className="space-y-5 p-4">
        <div className="flex flex-wrap items-start justify-between gap-3 border-b border-[var(--sandbox-console-border)] pb-4">
          <div className="flex min-w-0 items-start gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)]">
              <div className={cn('text-xl text-[var(--sandbox-console-brand)]', blueprint?.icon)} />
            </div>
            <div className="min-w-0">
              <h2 className="truncate font-display text-lg font-semibold text-[var(--sandbox-console-text)]">
                {blueprint?.name ?? entityLabel}
              </h2>
              <p className="mt-1 max-w-2xl text-sm leading-5 text-[var(--sandbox-console-muted)]">
                {blueprint?.description}
              </p>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <ConsoleChip tone={runtimeBackend === 'tee' ? 'warn' : 'ready'}>{runtimeLabel(runtimeBackend)}</ConsoleChip>
            {isTeeBlueprint ? <ConsoleChip tone="warn">TEE path</ConsoleChip> : null}
          </div>
        </div>

        <div className="grid gap-4 lg:grid-cols-[minmax(0,1.05fr)_minmax(280px,0.95fr)]">
          <div className="space-y-4">
            <LaunchInput
              label={`${entityLabel} Name`}
              inputRef={nameInputRef}
              value={valueString(values, 'name')}
              onChange={(event) => onChange('name', event.target.value)}
              placeholder={field(job, 'name')?.placeholder ?? 'agent-workspace'}
              error={nameError}
            />

            <LaunchInput
              label="Docker Image"
              value={selectedImage}
              onChange={(event) => onChange('image', event.target.value)}
              placeholder={field(job, 'image')?.placeholder ?? 'ghcr.io/tangle-network/blueprint-sidecar:all-harness'}
              list={imageListId}
            />
            <datalist id={imageListId}>
              {imageOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </datalist>
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
              options={fieldOptions(job, 'stack')}
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
          <div className="rounded-md border border-amber-400/20 bg-amber-400/10 px-3 py-2">
            <p className="text-xs leading-5 text-amber-200">
              This agent needs AI credentials to chat. Add them as environment variables now or inject them later through Secrets.
            </p>
          </div>
        ) : null}

        <div className="grid gap-4 border-t border-[var(--sandbox-console-border)] pt-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.7fr)]">
          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <p className="font-display text-xs font-semibold text-[var(--sandbox-console-secondary)]">Environment Variables</p>
              <span className="font-data text-[11px] text-[var(--sandbox-console-subtle)]">injected at boot</span>
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
      <ConsoleSection title="Deploy Summary">
        <div className="divide-y divide-[var(--sandbox-console-border)]">
          <SummaryRow
            label="Mode"
            value={selectedBlueprint ? entityLabel : 'Unselected'}
            detail={selectedBlueprint?.name ?? 'Choose a blueprint'}
            tone={selectedBlueprint ? 'brand' : 'warn'}
          />
          <SummaryRow
            label="Spec"
            value={step === 'deploy' ? 'Locked' : step === 'configure' ? 'Editing' : 'Open'}
            detail={step === 'deploy' ? 'ready for transaction' : 'mutable'}
            tone={step === 'deploy' ? 'ready' : 'muted'}
          />
          <SummaryRow
            label="Runtime"
            value={runtimeLabel(runtimeBackend)}
            detail={runtimeBackend === 'tee' ? 'attestation path' : 'standard path'}
            tone={runtimeBackend === 'tee' ? 'warn' : 'ready'}
          />
          <SummaryRow
            label="Capacity"
            value={formatCapacityValue(capacity)}
            detail="available slots"
            tone={capacity !== undefined && Number(capacity) === 0 ? 'warn' : 'ready'}
          />
          <SummaryRow
            label="Wallet"
            value={isConnected ? 'Connected' : isReconnectingWallet ? 'Syncing' : 'Offline'}
            detail={isConnected ? 'can sign' : 'deploy blocked'}
            tone={isConnected ? 'ready' : isReconnectingWallet ? 'warn' : 'danger'}
          />
          <SummaryRow
            label="Service"
            value={serviceState}
            detail={`blueprint ${infra.blueprintId || '--'} / service ${infra.serviceId || '--'}`}
            tone={serviceTone({ serviceValidating, serviceError, hasValidService, isNewService })}
          />
          <SummaryRow
            label="Operators"
            value={operatorSummary}
            detail={isNewService ? 'service quorum' : 'operator service'}
            tone={operatorsError ? 'warn' : 'brand'}
          />
          <SummaryRow
            label="Agent mode"
            value={agentIdentifier || 'Compute only'}
            detail={agentIdentifier ? 'chat enabled' : 'no bundled agent'}
            tone={agentIdentifier ? 'brand' : 'muted'}
          />
          <SummaryRow
            label="Network"
            value={ports.length > 0 ? `${ports.length} port${ports.length === 1 ? '' : 's'}` : 'Default'}
            detail={ports.length > 0 ? ports.join(', ') : 'operator proxy'}
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
  tone,
}: {
  label: string;
  value: string;
  detail: string;
  tone: ConsoleTone;
}) {
  return (
    <div className="grid gap-1 px-3 py-3">
      <div className="flex items-center justify-between gap-3">
        <span className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
          {label}
        </span>
        <ConsoleChip tone={tone}>{value}</ConsoleChip>
      </div>
      <p className="truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
        {detail}
      </p>
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
          options={BUNDLED_AGENT_OPTIONS}
        />
      ) : (
        <LaunchInput
          label="Agent"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={image ? 'default' : 'Choose an image first'}
        />
      )}
      <p className="mt-1.5 text-xs leading-5 text-[var(--sandbox-console-muted)]">
        {helpText}
      </p>
      {!usesBundledSelector && value.trim() !== '' && (
        <div className="mt-3 rounded-md border border-amber-400/20 bg-amber-400/10 px-3 py-2">
          <p className="text-xs leading-5 text-amber-200">
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
        className="w-full max-w-2xl overflow-hidden rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] shadow-[var(--sandbox-console-shadow-lg)]"
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
            className="flex h-8 w-8 items-center justify-center rounded-md text-[var(--sandbox-console-muted)] transition-colors hover:bg-[var(--sandbox-console-hover)] hover:text-[var(--sandbox-console-text)]"
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
                detail={teeRequiredLocked ? 'Pinned by TEE runtime' : 'Require attested hardware isolation.'}
              />
              <LaunchNativeSelect
                label="TEE Type"
                value={valueString(values, 'teeType', '0')}
                options={fieldOptions(job, 'teeType')}
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
  onViewDetail: () => void;
  onOpenInfra: () => void;
  onProvisionReady: (sandboxId: string, sidecarUrl: string) => void;
}

function DeployStep({
  blueprint, job, values, ports, infra, entityLabel, deploy,
  capacity, provisionEstimate, provisionPriceFormatted,
  hasProvisionRfq, priceLoading,
  serviceInfo, serviceValidating, serviceError,
  onBack, onDeploy, onViewDetail, onOpenInfra, onProvisionReady,
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
      {capacity !== undefined && Number(capacity) > 0 && (
        <div className="flex items-center gap-2 px-1">
          <div className="i-ph:shield-check text-sm text-teal-400" />
          <span className="text-xs text-cloud-elements-textTertiary">
            <span className="font-data font-semibold text-cloud-elements-textSecondary">{String(capacity)}</span> capacity slots available
          </span>
        </div>
      )}
      {capacity !== undefined && Number(capacity) === 0 && isSandbox && status === 'idle' && (
        <div className="rounded-xl border border-amber-500/20 bg-amber-500/[0.03] p-4">
          <div className="flex items-center gap-3">
            <div className="i-ph:warning-circle text-lg text-amber-400" />
            <div className="flex-1">
              <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                No capacity available
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mt-0.5">
                All operator slots are in use. Delete unused sandboxes or try again later.
              </p>
            </div>
          </div>
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
        <OperatorList
          operators={operators}
          operatorsLoading={operatorsLoading}
          operatorsError={operatorsError}
          operatorCount={operatorCount}
          blueprintId={infra.blueprintId}
        />
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
            <LaunchActionButton variant="secondary" size="sm" onClick={onOpenInfra}>Settings</LaunchActionButton>
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
        <LaunchActionButton variant="secondary" onClick={onBack}>Back</LaunchActionButton>
        {isComplete ? (
          <LaunchActionButton variant="success" onClick={onViewDetail}>
            <div className="i-ph:check-bold text-sm" />
            View {entityLabel}
          </LaunchActionButton>
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
  infra, serviceInfo, serviceValidating, serviceError, isInstanceMode,
}: {
  infra: { serviceId: string };
  serviceInfo: { active: boolean; permitted: boolean } | null;
  serviceValidating: boolean;
  serviceError: string | null;
  isInstanceMode: boolean;
}) {
  if (serviceValidating) {
    return (
      <>
        <div className="w-3 h-3 rounded-full border border-cloud-elements-textTertiary border-t-transparent animate-spin" />
        <span className="text-cloud-elements-textTertiary">Checking service...</span>
      </>
    );
  }
  if (isInstanceMode) {
    return (
      <>
        <div className="i-ph:plus-circle text-sm text-violet-400" />
        <span className="text-violet-400">New service</span>
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

function OperatorList({
  operators,
  operatorsLoading,
  operatorsError,
  operatorCount,
  blueprintId,
}: {
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  operatorsError?: Error | null;
  operatorCount: bigint;
  blueprintId: string;
}) {
  const titleCount = operatorsLoading
    ? '...'
    : operatorsError && operatorCount > 0n
      ? operatorCount.toString()
      : String(operators.length);

  return (
    <div className="glass-card rounded-xl p-4">
      <div className="flex items-center gap-2 mb-3">
        <div className="i-ph:users-three text-sm text-cloud-elements-textTertiary" />
        <span className="text-xs font-display font-medium text-cloud-elements-textSecondary">
          Operators ({titleCount})
        </span>
      </div>
      {operatorsLoading ? (
        <div className="flex items-center gap-2">
          <div className="w-3 h-3 rounded-full border border-cloud-elements-textTertiary border-t-transparent animate-spin" />
          <span className="text-xs text-cloud-elements-textTertiary">Discovering operators for blueprint #{blueprintId}...</span>
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
          <p className="text-[11px] text-cloud-elements-textTertiary">
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
    <LaunchActionButton size="lg" onClick={onDeploy} disabled={isDisabled}>
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
    </LaunchActionButton>
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
