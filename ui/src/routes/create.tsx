import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { Input, Select } from '@tangle-network/blueprint-ui/components';
import { InfrastructureModal } from '~/components/shared/InfrastructureModal';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { infraStore, updateInfra } from '@tangle-network/blueprint-ui';
import { BlueprintJobForm, type FormSection } from '@tangle-network/blueprint-ui/components';
import { Identicon } from '@tangle-network/blueprint-ui/components';
import {
  ConsoleChip,
  ConsoleMetricStrip,
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
import { getAllBlueprints, getBlueprint, type BlueprintDefinition, type JobDefinition } from '@tangle-network/blueprint-ui';
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

type ConsoleTone = NonNullable<ConsoleMetric['tone']>;

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

const PRE_AGENT_SECTIONS: FormSection[] = [
  { label: 'Identity', fields: ['name'] },
  { label: 'Image', fields: ['image'] },
];

function getPostAgentSections(isTee: boolean): FormSection[] {
  return [
    { label: 'Runtime & Stack', fields: ['runtimeBackend', 'stack'] },
    { label: 'Resources', fields: ['cpuCores', 'memoryMb', 'diskGb'] },
    { label: 'Timeouts', fields: ['maxLifetimeSeconds', 'idleTimeoutSeconds'] },
    { label: 'Features', fields: ['sshEnabled', 'sshPublicKey'] },
    { label: 'Advanced Options', fields: isTee ? ['metadataJson', 'teeRequired', 'teeType'] : ['metadataJson'], collapsed: true },
  ];
}

// ── Wizard Steps ──

type WizardStep = 'blueprint' | 'configure' | 'deploy';

const STEPS: { key: WizardStep; label: string; icon: string }[] = [
  { key: 'blueprint', label: 'Mode', icon: 'i-ph:cube' },
  { key: 'configure', label: 'Spec', icon: 'i-ph:gear' },
  { key: 'deploy', label: 'Launch', icon: 'i-ph:rocket-launch' },
];

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

function formatCapacityValue(value: number | bigint | undefined) {
  if (value == null) return '--';
  return typeof value === 'bigint' ? value.toString() : String(value);
}

function runtimeLabel(value: string) {
  if (value === 'firecracker') return 'Firecracker';
  if (value === 'tee') return 'TEE';
  return 'Docker';
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
  const postAgentSections = useMemo(() => getPostAgentSections(isTeeBlueprint), [isTeeBlueprint]);

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

  // Extra ports input (not an ABI field — merged into metadataJson before deploy)
  const [portsInput, setPortsInput] = useState('');
  const allHarnessEnabled = parseCapabilitiesJson(values.capabilitiesJson).has('all_harness');
  const runtimeBackend = String(values.runtimeBackend || 'docker').toLowerCase();
  const supportsMetadataPorts = runtimeBackend !== 'firecracker';
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
      capabilitiesJson: allHarnessEnabled ? JSON.stringify(['all_harness']) : JSON.stringify([]),
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
  }, [runtimeBackend, supportsMetadataPorts, values, portsInput, allHarnessEnabled]);

  // Unified deploy hook — manages both submitJob and requestService paths
  const deploy = useCreateDeploy({ blueprint: selectedBlueprint, job: createJob, values: mergedValues, infra, validate, capacity });
  const { reset: deployReset } = deploy;

  const isSandbox = deploy.mode === 'sandbox';
  const entityLabel = isSandbox ? 'Sandbox' : 'Instance';
  const currentIdx = STEPS.findIndex((s) => s.key === step);
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
    const mapping = BLUEPRINT_INFRA[bp.id];
    if (mapping) {
      updateInfra({ blueprintId: mapping.blueprintId, serviceId: mapping.serviceId, serviceValidated: false });
      if (mapping.serviceId) {
        validateService(BigInt(mapping.serviceId), address);
      }
    }
    setStep('configure');
  }, [resetForm, deployReset, address, validateService]);

  const showConnectPanel = !isConnected && !address && !isReconnectingWallet;
  const parsedPorts = supportsMetadataPorts ? parsePortsInput(portsInput) : [];
  const launchMetrics: ConsoleMetric[] = [
    {
      label: 'Blueprint',
      value: selectedBlueprint ? entityLabel : 'Select',
      detail: selectedBlueprint?.name ?? 'catalog',
      tone: selectedBlueprint ? 'brand' : 'muted',
    },
    {
      label: 'Runtime',
      value: runtimeLabel(runtimeBackend),
      detail: step === 'blueprint' ? 'pending' : 'backend',
      tone: runtimeBackend === 'tee' ? 'warn' : 'ready',
    },
    {
      label: 'Capacity',
      value: formatCapacityValue(capacity),
      detail: 'slots',
      tone: capacity !== undefined && Number(capacity) === 0 ? 'warn' : 'ready',
    },
    {
      label: 'Service',
      value: serviceValidating
        ? 'Checking'
        : serviceError
          ? 'Blocked'
          : deploy.isNewService
            ? 'New'
            : deploy.hasValidService
              ? 'Verified'
              : 'Pending',
      detail: `#${infra.serviceId || '--'}`,
      tone: serviceTone({
        serviceValidating,
        serviceError,
        hasValidService: deploy.hasValidService,
        isNewService: deploy.isNewService,
      }),
    },
  ];

  return (
    <ConsolePage
      title="Launch Console"
      eyebrow="Tangle sandbox compiler"
      actions={step !== 'blueprint' ? (
        <Button variant="secondary" onClick={() => setShowInfra(true)}>
          <span className="i-ph:sliders-horizontal text-base" />
          Infrastructure
        </Button>
      ) : null}
    >
      <div className="grid min-h-full gap-4 xl:grid-cols-[260px_minmax(0,1fr)_320px]">
        <aside className="space-y-4">
          <LaunchModeRail
            blueprints={blueprints}
            selectedBlueprint={selectedBlueprint}
            onSelect={handleSelectBlueprint}
          />
          <LaunchPhaseRail
            step={step}
            currentIdx={currentIdx}
            onStepChange={(nextStep) => setStep(nextStep)}
          />
        </aside>

        <main className="min-w-0 space-y-4">
          {showConnectPanel && (
            <ConnectWalletPanel
              description="Provisioning a sandbox or instance requires a connected wallet on Tangle Network. You can browse blueprints below, but deploying will be blocked until you connect."
            />
          )}

          <ConsoleMetricStrip metrics={launchMetrics} />

          {step === 'blueprint' && (
            <ConsoleSection title="Blueprint Catalog">
              <BlueprintSelector blueprints={blueprints} onSelect={handleSelectBlueprint} />
            </ConsoleSection>
          )}

          {step === 'configure' && createJob && displayJob && (
            <div className="space-y-4">
              <ConsoleSection title={`${entityLabel} Spec`}>
                <div className="p-4">
                  <div className="mb-4 flex flex-wrap items-start justify-between gap-3 border-b border-[var(--sandbox-console-border)] pb-4">
                    <div className="flex min-w-0 items-start gap-3">
                      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)]">
                        <div className={cn('text-xl text-[var(--sandbox-console-brand)]', selectedBlueprint?.icon)} />
                      </div>
                      <div className="min-w-0">
                        <h2 className="truncate font-display text-lg font-semibold text-[var(--sandbox-console-text)]">
                          {selectedBlueprint?.name ?? entityLabel}
                        </h2>
                        <p className="mt-1 text-sm text-[var(--sandbox-console-muted)]">
                          {selectedBlueprint?.description}
                        </p>
                      </div>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <ConsoleChip tone="brand">service #{infra.serviceId || '--'}</ConsoleChip>
                      <ConsoleChip tone={runtimeBackend === 'tee' ? 'warn' : 'ready'}>{runtimeLabel(runtimeBackend)}</ConsoleChip>
                    </div>
                  </div>

                  <div className="space-y-1">
                    <BlueprintJobForm
                      job={displayJob}
                      values={values}
                      onChange={onChange}
                      errors={errors}
                      sections={PRE_AGENT_SECTIONS}
                    />

                    {supportsAgentConfiguration && (
                      <AgentConfigurationField
                        image={selectedImage}
                        value={configuredAgentIdentifier}
                        usesBundledSelector={usesBundledAgentSelector}
                        onChange={(next) => onChange('agentIdentifier', next)}
                      />
                    )}

                    <AllHarnessCapabilityField
                      enabled={allHarnessEnabled}
                      onChange={(enabled) => onChange('capabilitiesJson', enabled ? JSON.stringify(['all_harness']) : JSON.stringify([]))}
                    />

                    {configuredAgentIdentifier && (
                      <div className="mt-4 rounded-md border border-amber-500/20 bg-amber-500/5 px-3 py-2">
                        <p className="text-xs text-amber-300">
                          This agent needs AI credentials to chat. You can add them now in Environment Variables below, or inject them later in the Secrets tab.
                        </p>
                      </div>
                    )}

                    <BlueprintJobForm
                      job={displayJob}
                      values={values}
                      onChange={onChange}
                      errors={errors}
                      sections={postAgentSections}
                    />

                    <div className="mt-6 space-y-1.5 border-t border-cloud-elements-dividerColor pt-4">
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

                    <div className="mt-6 space-y-1.5 border-t border-cloud-elements-dividerColor pt-4">
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
                          'w-full rounded-md border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 font-data text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary transition-colors focus:border-cloud-elements-borderColorActive focus:outline-none',
                          !supportsMetadataPorts && 'cursor-not-allowed opacity-60',
                        )}
                      />
                      <p className="text-[11px] text-cloud-elements-textTertiary">
                        {supportsMetadataPorts
                          ? 'Comma-separated container ports to expose through the operator API proxy.'
                          : 'Firecracker backend currently does not support metadata_json.ports mappings.'}
                      </p>
                    </div>
                  </div>
                </div>
              </ConsoleSection>
              <div className="flex justify-between">
                <Button variant="secondary" onClick={() => setStep('blueprint')}>Back</Button>
                <Button onClick={() => { if (validate()) setStep('deploy'); }} disabled={!createJob || !values.name}>Continue</Button>
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

        <LaunchReadinessRail
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

      <InfrastructureModal open={showInfra} onOpenChange={setShowInfra} />
    </ConsolePage>
  );
}

function LaunchModeRail({
  blueprints,
  selectedBlueprint,
  onSelect,
}: {
  blueprints: BlueprintDefinition[];
  selectedBlueprint?: BlueprintDefinition;
  onSelect: (bp: BlueprintDefinition) => void;
}) {
  return (
    <ConsoleSection title="Launch Modes">
      <div className="grid gap-px bg-[var(--sandbox-console-border)] p-px">
        {blueprints.map((bp) => {
          const isActive = selectedBlueprint?.id === bp.id;
          return (
            <button
              key={bp.id}
              onClick={() => onSelect(bp)}
              className={cn(
                'flex min-h-20 items-start gap-3 bg-[var(--sandbox-console-panel)] p-3 text-left transition-colors hover:bg-[var(--sandbox-console-hover)]',
                isActive && 'bg-[var(--sandbox-console-brand-soft)]',
              )}
            >
              <span className={cn('mt-0.5 text-lg text-[var(--sandbox-console-brand)]', bp.icon)} />
              <span className="min-w-0">
                <span className="block truncate font-display text-sm font-semibold text-[var(--sandbox-console-text)]">
                  {bp.name}
                </span>
                <span className="mt-1 line-clamp-2 block text-xs leading-5 text-[var(--sandbox-console-muted)]">
                  {bp.description}
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </ConsoleSection>
  );
}

function LaunchPhaseRail({
  step,
  currentIdx,
  onStepChange,
}: {
  step: WizardStep;
  currentIdx: number;
  onStepChange: (step: WizardStep) => void;
}) {
  return (
    <ConsoleSection title="Compiler Phases">
      <div className="divide-y divide-[var(--sandbox-console-border)]">
        {STEPS.map((phase, index) => {
          const isCurrent = phase.key === step;
          const isAvailable = index <= currentIdx;
          return (
            <button
              key={phase.key}
              onClick={() => {
                if (isAvailable) onStepChange(phase.key);
              }}
              className={cn(
                'flex h-12 w-full items-center gap-3 px-3 text-left transition-colors',
                isCurrent ? 'bg-[var(--sandbox-console-brand-soft)]' : 'hover:bg-[var(--sandbox-console-hover)]',
                !isAvailable && 'cursor-default opacity-50 hover:bg-transparent',
              )}
            >
              <span className={cn('text-base', phase.icon, isCurrent ? 'text-[var(--sandbox-console-brand)]' : 'text-[var(--sandbox-console-muted)]')} />
              <span className="font-data text-xs uppercase tracking-[0.12em] text-[var(--sandbox-console-text)]">
                {phase.label}
              </span>
              {index < currentIdx ? (
                <span className="ml-auto i-ph:check text-sm text-[var(--sandbox-console-success)]" />
              ) : null}
            </button>
          );
        })}
      </div>
    </ConsoleSection>
  );
}

function LaunchReadinessRail({
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
      <ConsoleSection title="Launch Readiness">
        <div className="divide-y divide-[var(--sandbox-console-border)]">
          <ReadinessRow
            label="Mode"
            value={selectedBlueprint ? entityLabel : 'Unselected'}
            detail={selectedBlueprint?.name ?? 'Choose a blueprint'}
            tone={selectedBlueprint ? 'brand' : 'warn'}
          />
          <ReadinessRow
            label="Spec"
            value={step === 'deploy' ? 'Locked' : step === 'configure' ? 'Editing' : 'Open'}
            detail={step === 'deploy' ? 'ready for transaction' : 'mutable'}
            tone={step === 'deploy' ? 'ready' : 'muted'}
          />
          <ReadinessRow
            label="Runtime"
            value={runtimeLabel(runtimeBackend)}
            detail={runtimeBackend === 'tee' ? 'attestation path' : 'standard path'}
            tone={runtimeBackend === 'tee' ? 'warn' : 'ready'}
          />
          <ReadinessRow
            label="Capacity"
            value={formatCapacityValue(capacity)}
            detail="available slots"
            tone={capacity !== undefined && Number(capacity) === 0 ? 'warn' : 'ready'}
          />
          <ReadinessRow
            label="Wallet"
            value={isConnected ? 'Connected' : isReconnectingWallet ? 'Syncing' : 'Offline'}
            detail={isConnected ? 'can sign' : 'deploy blocked'}
            tone={isConnected ? 'ready' : isReconnectingWallet ? 'warn' : 'danger'}
          />
          <ReadinessRow
            label="Service"
            value={serviceState}
            detail={`blueprint ${infra.blueprintId || '--'} / service ${infra.serviceId || '--'}`}
            tone={serviceTone({ serviceValidating, serviceError, hasValidService, isNewService })}
          />
          <ReadinessRow
            label="Operators"
            value={operatorSummary}
            detail={isNewService ? 'service quorum' : 'operator service'}
            tone={operatorsError ? 'warn' : 'brand'}
          />
          <ReadinessRow
            label="Agent mode"
            value={agentIdentifier || 'Compute only'}
            detail={agentIdentifier ? 'chat enabled' : 'no bundled agent'}
            tone={agentIdentifier ? 'brand' : 'muted'}
          />
          <ReadinessRow
            label="Network"
            value={ports.length > 0 ? `${ports.length} port${ports.length === 1 ? '' : 's'}` : 'Default'}
            detail={ports.length > 0 ? ports.join(', ') : 'operator proxy'}
            tone={ports.length > 0 ? 'brand' : 'muted'}
          />
        </div>
        <div className="border-t border-[var(--sandbox-console-border)] p-3">
          <Button variant="secondary" size="sm" className="w-full justify-center" onClick={onOpenInfra}>
            <span className="i-ph:sliders-horizontal text-sm" />
            Infrastructure
          </Button>
        </div>
      </ConsoleSection>
    </aside>
  );
}

function ReadinessRow({
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
    ? 'Choose an agent already bundled in this image. “None” keeps the resource compute-only and hides chat.'
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
      {!usesBundledSelector && value.trim() !== '' && (
        <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2">
          <p className="text-xs text-amber-300">
            Custom agent identifiers depend on the selected image registering the agent
            internally. If the image doesn't recognize this name, chat will fail with a 502
            once the sandbox is running. Smoke-test the agent endpoint after provision.
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
    <div className="mt-6 pt-4 border-t border-cloud-elements-dividerColor">
      <label className="flex items-start gap-3">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onChange(e.target.checked)}
          className="mt-0.5 h-4 w-4 rounded border-cloud-elements-borderColor bg-cloud-elements-background-depth-2"
        />
        <span className="space-y-1">
          <span className="block text-xs font-display font-medium text-cloud-elements-textSecondary">
            All-Harness Runtime
          </span>
          <span className="block text-[11px] text-cloud-elements-textTertiary">
            Request the open-source runtime with Claude, Codex, opencode, Kimi, and Gemini harnesses available in the sandbox image.
          </span>
        </span>
      </label>
    </div>
  );
}

// ── Blueprint Selector ──

function BlueprintSelector({
  blueprints,
  onSelect,
}: {
  blueprints: BlueprintDefinition[];
  onSelect: (bp: BlueprintDefinition) => void;
}) {
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
    <div className="grid gap-px bg-[var(--sandbox-console-border)] p-px">
      {blueprints.map((bp) => (
        <button
          key={bp.id}
          onClick={() => onSelect(bp)}
          className={cn(
            'bg-[var(--sandbox-console-panel)] p-4 text-left transition-all cursor-pointer',
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
              <p className="text-sm text-cloud-elements-textSecondary">{bp.description}</p>
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
          <Button variant="success" onClick={onViewDetail}>
            <div className="i-ph:check-bold text-sm" />
            View {entityLabel}
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
