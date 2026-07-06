import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import {
  cn,
  getAllBlueprints,
  getBlueprint,
  infraStore,
  updateInfra,
  useJobForm,
  useJobPrice,
  useServiceValidation,
  type BlueprintDefinition,
  type JobDefinition,
} from '@tangle-network/blueprint-ui';
import { ConsolePage, ConsoleSection } from '~/components/console/ConsolePrimitives';
import { InfrastructureModal } from '~/components/shared/InfrastructureModal';
import { ConnectWalletPanel } from '~/components/shared/ConnectWalletPanel';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { useCreateDeploy } from '~/lib/hooks/useCreateDeploy';
import { updateSandboxStatus } from '~/lib/stores/sandboxes';
import { updateInstanceStatus } from '~/lib/stores/instances';
import {
  isBundledSandboxImage,
  normalizeAgentIdentifier,
  sanitizeBundledAgentIdentifier,
} from '~/lib/agents';
import {
  BLUEPRINT_INFRA,
  parseCapabilitiesJson,
  parsePortsInput,
  type ServiceSetupMode,
  type WizardStep,
} from '~/components/create/support';
import { LaunchActionButton } from '~/components/create/launch-fields';
import {
  AdvancedOptionsModal,
  LaunchModeStrip,
  LaunchSpecComposer,
  LaunchSummaryPanel,
} from '~/components/create/launch-spec';
import { DeployStep } from '~/components/create/deploy-step';

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
      actions={step === 'configure' ? (
        <LaunchActionButton variant="secondary" onClick={() => openInfra('existing')}>
          <span className="i-ph:sliders-horizontal text-base" />
          Infrastructure
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
              validateService={validateService}
              onBack={() => { setStep('configure'); deployReset(); }}
              onDeploy={deploy.deploy}
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
