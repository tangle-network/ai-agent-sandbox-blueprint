import { useEffect, type RefObject } from 'react';
import { cn, type BlueprintDefinition, type JobDefinition } from '@tangle-network/blueprint-ui';
import { ConsoleChip, ConsoleSection } from '~/components/console/ConsolePrimitives';
import {
  IdentityMark,
  getAgentIdentity,
  getBlueprintIdentity,
  getCapabilityIdentity,
  getOperatorIdentity,
  getResourceIdentity,
  getRuntimeIdentity,
  getStackIdentity,
  type IdentityMeta,
} from '~/components/shared/VisualIdentity';
import { EnvEditor } from '~/components/shared/EnvEditor';
import {
  AgentConfigurationField,
  AllHarnessCapabilityField,
  ComputerUseCapabilityField,
  LaunchActionButton,
  LaunchImageSelect,
  LaunchInput,
  LaunchNativeSelect,
  LaunchTextArea,
  LaunchToggle,
  ResourceSizingControls,
  SegmentedControl,
} from './launch-fields';
import {
  executionMetricToneClass,
  field,
  fieldOptions,
  formatCapacityValue,
  hoursFromSeconds,
  minutesFromSeconds,
  runtimeLabel,
  serviceTone,
  setCapabilityJson,
  valueString,
  type ConsoleTone,
  type WizardStep,
} from './support';

export function LaunchModeStrip({
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

export function LaunchSpecComposer({
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

export function LaunchSummaryPanel({
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
      </ConsoleSection>
    </aside>
  );
}

export function SummaryRow({
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

export function AdvancedOptionsModal({
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
