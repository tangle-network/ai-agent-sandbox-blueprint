import { useState, useCallback, useMemo } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Badge } from '~/components/ui/badge';
import { InfrastructureModal, InfraBar } from '~/components/shared/InfrastructureModal';
import { infraStore } from '~/lib/stores/infra';
import { BlueprintJobForm, type FormSection } from '~/components/forms/BlueprintJobForm';
import { FormSummary } from '~/components/forms/FormSummary';
import { useJobForm } from '~/lib/hooks/useJobForm';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getAllBlueprints, getBlueprint, type BlueprintDefinition, type JobDefinition } from '~/lib/blueprints';
import { addSandbox } from '~/lib/stores/sandboxes';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { cn } from '~/lib/utils';

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
  const { submitJob, status: txStatus, error: txError, txHash, reset: resetTx } = useSubmitJob();
  const { data: capacity } = useAvailableCapacity();

  // Pre-select from query params
  const preselectedId = searchParams.get('blueprint');
  const preselected = preselectedId ? getBlueprint(preselectedId) : undefined;

  const [selectedBlueprint, setSelectedBlueprint] = useState<BlueprintDefinition | undefined>(preselected);
  const [step, setStep] = useState<WizardStep>(preselected ? 'configure' : 'blueprint');
  const [showInfra, setShowInfra] = useState(false);
  const [provisionCallId, setProvisionCallId] = useState<number | null>(null);

  // The create/provision job is always job 0 for all blueprints
  const createJob = useMemo<JobDefinition | null>(
    () => selectedBlueprint?.jobs.find((j) => j.id === 0) ?? null,
    [selectedBlueprint],
  );

  const { values, errors, onChange, validate, reset: resetForm } = useJobForm(createJob);

  const currentIdx = STEPS.findIndex((s) => s.key === step);
  const canDeploy = createJob && values.name && infra.serviceId;

  const isSandbox = selectedBlueprint?.id === 'ai-agent-sandbox-blueprint';
  const entityLabel = isSandbox ? 'Sandbox' : 'Instance';

  // ── Handlers ──

  const handleSelectBlueprint = useCallback((bp: BlueprintDefinition) => {
    setSelectedBlueprint(bp);
    resetForm();
    resetTx();
    setStep('configure');
  }, [resetForm, resetTx]);

  const handleDeploy = useCallback(async () => {
    if (!createJob || !validate()) return;

    const args = encodeJobArgs(createJob, values);
    const name = String(values.name || '');
    const hash = await submitJob({
      serviceId: BigInt(infra.serviceId || '0'),
      jobId: createJob.id,
      args,
      label: `${createJob.label}: ${name}`,
    });

    if (hash && isSandbox) {
      addSandbox({
        id: name,
        name,
        image: String(values.image || ''),
        cpuCores: Number(values.cpuCores) || 2,
        memoryMb: Number(values.memoryMb) || 2048,
        diskGb: Number(values.diskGb) || 10,
        createdAt: Date.now(),
        blueprintId: infra.blueprintId,
        serviceId: infra.serviceId,
        status: 'creating',
        txHash: hash,
      });
    }
  }, [createJob, values, infra, submitJob, validate, isSandbox]);

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
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Review & Deploy</CardTitle>
              <CardDescription>Confirm your configuration and submit the on-chain transaction</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              {/* Summary */}
              <div className="glass-card rounded-lg p-4 space-y-2.5">
                <SummaryRow label="Blueprint" value={selectedBlueprint?.name ?? `#${infra.blueprintId}`} />
                <SummaryRow label="Service" value={`#${infra.serviceId}`} />
                <div className="border-t border-cloud-elements-dividerColor my-2" />
              </div>
              <FormSummary job={createJob} values={values} />

              {/* Pricing */}
              <div className="glass-card rounded-lg p-4">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm font-display font-medium text-cloud-elements-textSecondary">Estimated Cost</p>
                    <p className="text-xs text-cloud-elements-textTertiary mt-0.5">
                      {createJob.pricingMultiplier}x base rate
                    </p>
                  </div>
                  <Badge variant="accent">{createJob.label}</Badge>
                </div>
              </div>

              {/* Capacity */}
              {capacity !== undefined && (
                <div className="glass-card rounded-lg p-3">
                  <div className="flex items-center gap-2">
                    <div className="i-ph:shield-check text-sm text-teal-400" />
                    <span className="text-xs text-cloud-elements-textSecondary">
                      Available capacity: <span className="font-data font-semibold">{String(capacity)}</span> slots
                    </span>
                  </div>
                </div>
              )}

              {/* TX Status */}
              {txStatus !== 'idle' && (
                <div className={cn(
                  'glass-card rounded-lg p-4',
                  txStatus === 'confirmed' && 'border-teal-500/30',
                  txStatus === 'failed' && 'border-crimson-500/30',
                )}>
                  <div className="flex items-center gap-3">
                    {txStatus === 'signing' && <div className="i-ph:circle-fill text-sm text-amber-400 animate-pulse" />}
                    {txStatus === 'pending' && <div className="i-ph:circle-fill text-sm text-blue-400 animate-pulse" />}
                    {txStatus === 'confirmed' && <div className="i-ph:check-circle-fill text-sm text-teal-400" />}
                    {txStatus === 'failed' && <div className="i-ph:x-circle-fill text-sm text-crimson-400" />}
                    <div>
                      <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                        {txStatus === 'signing' && 'Waiting for wallet signature...'}
                        {txStatus === 'pending' && 'Transaction submitted — waiting for confirmation...'}
                        {txStatus === 'confirmed' && `${entityLabel} creation submitted!`}
                        {txStatus === 'failed' && 'Transaction failed'}
                      </p>
                      {txHash && (
                        <p className="text-xs font-data text-cloud-elements-textTertiary mt-0.5 truncate max-w-xs">
                          TX: {txHash}
                        </p>
                      )}
                      {txError && <p className="text-xs text-crimson-400 mt-0.5">{txError}</p>}
                    </div>
                  </div>
                </div>
              )}

              {/* Provision Progress */}
              {txStatus === 'confirmed' && provisionCallId && (
                <ProvisionProgress callId={provisionCallId} onReady={() => {}} />
              )}

              {!address && (
                <div className="glass-card rounded-lg p-4 border-amber-500/30">
                  <div className="flex items-center gap-3">
                    <div className="i-ph:wallet text-lg text-amber-400" />
                    <p className="text-sm text-cloud-elements-textSecondary">Connect your wallet to deploy</p>
                  </div>
                </div>
              )}
            </CardContent>
          </Card>
          <div className="flex justify-between">
            <Button variant="secondary" onClick={() => { setStep('configure'); resetTx(); }}>Back</Button>
            {txStatus === 'confirmed' ? (
              <Button variant="success" onClick={() => navigate('/sandboxes')}>
                <div className="i-ph:check-bold text-base" />
                View {entityLabel}s
              </Button>
            ) : (
              <Button
                size="lg"
                onClick={handleDeploy}
                disabled={!address || txStatus === 'signing' || txStatus === 'pending'}
              >
                {(txStatus === 'signing' || txStatus === 'pending') ? (
                  <>
                    <div className="i-ph:circle-fill text-sm animate-pulse" />
                    Deploying...
                  </>
                ) : (
                  <>
                    <div className="i-ph:lightning text-base" />
                    Deploy {entityLabel}
                  </>
                )}
              </Button>
            )}
          </div>
        </div>
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
              </div>
            </div>
            <div className="i-ph:arrow-right text-lg text-cloud-elements-textTertiary" />
          </div>
        </button>
      ))}
    </div>
  );
}

// ── Summary Row ──

function SummaryRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between text-sm">
      <span className="text-cloud-elements-textSecondary">{label}</span>
      <span className={cn('text-cloud-elements-textPrimary', mono ? 'font-data text-xs' : 'font-display')}>
        {value || '--'}
      </span>
    </div>
  );
}
