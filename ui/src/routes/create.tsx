import { useState, useCallback } from 'react';
import { useNavigate } from 'react-router';
import { useAccount } from 'wagmi';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Input } from '~/components/ui/input';
import { Badge } from '~/components/ui/badge';
import { Select } from '~/components/ui/select';
import { InfrastructureModal, InfraBar } from '~/components/shared/InfrastructureModal';
import { infraStore } from '~/lib/stores/infra';
import '~/lib/blueprints/sandbox-blueprint'; // auto-register
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { useAvailableCapacity } from '~/lib/hooks/useSandboxReads';
import { encodeSandboxCreate } from '~/lib/contracts/encoding';
import { JOB_IDS, PRICING_TIERS, type SandboxCreateParams } from '~/lib/types/sandbox';
import { addSandbox, updateSandboxStatus } from '~/lib/stores/sandboxes';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { cn } from '~/lib/utils';

type WizardStep = 'configure' | 'deploy';

const steps: { key: WizardStep; label: string; icon: string }[] = [
  { key: 'configure', label: 'Configure', icon: 'i-ph:gear' },
  { key: 'deploy', label: 'Deploy', icon: 'i-ph:lightning' },
];

export default function CreateSandbox() {
  const navigate = useNavigate();
  const { address } = useAccount();
  const infra = useStore(infraStore);
  const { submitJob, status: txStatus, error: txError, txHash, reset: resetTx } = useSubmitJob();
  const { data: capacity } = useAvailableCapacity();

  const [step, setStep] = useState<WizardStep>('configure');
  const [showInfra, setShowInfra] = useState(false);
  const [provisionCallId, setProvisionCallId] = useState<number | null>(null);

  // Sandbox config fields
  const [name, setName] = useState('');
  const [image, setImage] = useState('ubuntu:22.04');
  const [stack, setStack] = useState('default');
  const [agentIdentifier, setAgentIdentifier] = useState('');
  const [cpuCores, setCpuCores] = useState(2);
  const [memoryMb, setMemoryMb] = useState(2048);
  const [diskGb, setDiskGb] = useState(10);
  const [maxLifetime, setMaxLifetime] = useState(86400);
  const [idleTimeout, setIdleTimeout] = useState(3600);
  const [sshEnabled, setSshEnabled] = useState(false);
  const [sshPublicKey, setSshPublicKey] = useState('');
  const [webTerminalEnabled, setWebTerminalEnabled] = useState(true);
  const [envJson, setEnvJson] = useState('{}');
  const [metadataJson, setMetadataJson] = useState('{}');

  const currentIdx = steps.findIndex((s) => s.key === step);
  const canDeploy = !!name && !!infra.serviceId;
  const pricingTier = PRICING_TIERS[JOB_IDS.SANDBOX_CREATE];

  const handleDeploy = useCallback(async () => {
    if (!name) return;

    const params: SandboxCreateParams = {
      name,
      image,
      stack,
      agentIdentifier: agentIdentifier || name,
      envJson,
      metadataJson,
      sshEnabled,
      sshPublicKey,
      webTerminalEnabled,
      maxLifetimeSeconds: maxLifetime,
      idleTimeoutSeconds: idleTimeout,
      cpuCores,
      memoryMb,
      diskGb,
    };

    const args = encodeSandboxCreate(params);
    const hash = await submitJob({
      serviceId: BigInt(infra.serviceId || '0'),
      jobId: JOB_IDS.SANDBOX_CREATE,
      args,
      label: `Create Sandbox: ${name}`,
    });

    if (hash) {
      addSandbox({
        id: name,
        name,
        image,
        cpuCores,
        memoryMb,
        diskGb,
        createdAt: Date.now(),
        blueprintId: infra.blueprintId,
        serviceId: infra.serviceId,
        status: 'creating',
        txHash: hash,
      });
    }
  }, [name, image, stack, agentIdentifier, envJson, metadataJson, sshEnabled, sshPublicKey, webTerminalEnabled, maxLifetime, idleTimeout, cpuCores, memoryMb, diskGb, infra, submitJob]);

  return (
    <AnimatedPage className="mx-auto max-w-3xl px-4 sm:px-6 py-8">
      <div className="mb-6">
        <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Create Sandbox</h1>
        <p className="text-sm text-cloud-elements-textSecondary mt-1">Provision a new AI agent sandbox on Tangle Network</p>
      </div>

      {/* Infrastructure bar — auto-defaults from env, modal to change */}
      <InfraBar onOpenModal={() => setShowInfra(true)} />
      <InfrastructureModal open={showInfra} onOpenChange={setShowInfra} />

      {/* Step Indicator */}
      <div className="flex items-center gap-2 mb-8">
        {steps.map((s, i) => (
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
            {i < steps.length - 1 && <div className="w-4 h-px bg-cloud-elements-dividerColor shrink-0" />}
          </div>
        ))}
      </div>

      {/* Step 1: Configure */}
      {step === 'configure' && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Sandbox Configuration</CardTitle>
              <CardDescription>Configure your sandbox resources and settings</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              {/* Identity */}
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Name *</label>
                  <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="my-agent-sandbox" />
                </div>
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Agent ID</label>
                  <Input value={agentIdentifier} onChange={(e) => setAgentIdentifier(e.target.value)} placeholder={name || 'agent-1'} />
                </div>
              </div>

              {/* Image / Stack */}
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Docker Image</label>
                  <Input value={image} onChange={(e) => setImage(e.target.value)} placeholder="ubuntu:22.04" />
                </div>
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Stack</label>
                  <Select
                    value={stack}
                    onChange={(e) => setStack(e.target.value)}
                    options={[
                      { label: 'Default', value: 'default' },
                      { label: 'Python', value: 'python' },
                      { label: 'Node.js', value: 'nodejs' },
                      { label: 'Rust', value: 'rust' },
                    ]}
                  />
                </div>
              </div>

              {/* Resources */}
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-3">Resources</label>
                <div className="grid grid-cols-3 gap-4">
                  <div>
                    <label className="block text-xs text-cloud-elements-textTertiary mb-1">CPU Cores</label>
                    <Input type="number" value={cpuCores} onChange={(e) => setCpuCores(Number(e.target.value))} min={1} max={16} />
                  </div>
                  <div>
                    <label className="block text-xs text-cloud-elements-textTertiary mb-1">Memory (MB)</label>
                    <Input type="number" value={memoryMb} onChange={(e) => setMemoryMb(Number(e.target.value))} min={256} max={32768} step={256} />
                  </div>
                  <div>
                    <label className="block text-xs text-cloud-elements-textTertiary mb-1">Disk (GB)</label>
                    <Input type="number" value={diskGb} onChange={(e) => setDiskGb(Number(e.target.value))} min={1} max={100} />
                  </div>
                </div>
              </div>

              {/* Timeouts */}
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Max Lifetime (s)</label>
                  <Input type="number" value={maxLifetime} onChange={(e) => setMaxLifetime(Number(e.target.value))} min={0} />
                  <p className="text-xs text-cloud-elements-textTertiary mt-1">0 = unlimited</p>
                </div>
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Idle Timeout (s)</label>
                  <Input type="number" value={idleTimeout} onChange={(e) => setIdleTimeout(Number(e.target.value))} min={0} />
                </div>
              </div>

              {/* Toggles */}
              <div className="space-y-3">
                <div className="flex items-center gap-3">
                  <button
                    onClick={() => setWebTerminalEnabled(!webTerminalEnabled)}
                    className={cn('relative w-11 h-6 rounded-full transition-colors', webTerminalEnabled ? 'bg-violet-600' : 'bg-cloud-elements-background-depth-4')}
                  >
                    <span className={cn('absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform', webTerminalEnabled && 'translate-x-5')} />
                  </button>
                  <span className="text-sm font-display text-cloud-elements-textSecondary">Web Terminal</span>
                </div>
                <div className="flex items-center gap-3">
                  <button
                    onClick={() => setSshEnabled(!sshEnabled)}
                    className={cn('relative w-11 h-6 rounded-full transition-colors', sshEnabled ? 'bg-violet-600' : 'bg-cloud-elements-background-depth-4')}
                  >
                    <span className={cn('absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform', sshEnabled && 'translate-x-5')} />
                  </button>
                  <span className="text-sm font-display text-cloud-elements-textSecondary">Enable SSH</span>
                </div>
              </div>

              {sshEnabled && (
                <div>
                  <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">SSH Public Key</label>
                  <textarea
                    value={sshPublicKey}
                    onChange={(e) => setSshPublicKey(e.target.value)}
                    placeholder="ssh-ed25519 AAAA..."
                    className="flex min-h-[80px] w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
                  />
                </div>
              )}

              {/* Advanced */}
              <details className="group">
                <summary className="cursor-pointer text-sm font-display font-medium text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary transition-colors">
                  Advanced Options
                </summary>
                <div className="mt-4 space-y-4">
                  <div>
                    <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Environment (JSON)</label>
                    <textarea
                      value={envJson}
                      onChange={(e) => setEnvJson(e.target.value)}
                      placeholder='{}'
                      rows={3}
                      className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Metadata (JSON)</label>
                    <textarea
                      value={metadataJson}
                      onChange={(e) => setMetadataJson(e.target.value)}
                      placeholder='{}'
                      rows={3}
                      className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
                    />
                  </div>
                </div>
              </details>
            </CardContent>
          </Card>
          <div className="flex justify-end">
            <Button onClick={() => setStep('deploy')} disabled={!canDeploy}>Continue</Button>
          </div>
        </div>
      )}

      {/* Step 2: Deploy */}
      {step === 'deploy' && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Review & Deploy</CardTitle>
              <CardDescription>Confirm your sandbox configuration and submit the on-chain transaction</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              {/* Config Summary */}
              <div className="glass-card rounded-lg p-4 space-y-2.5">
                <SummaryRow label="Blueprint" value={`#${infra.blueprintId}`} />
                <SummaryRow label="Service" value={`#${infra.serviceId}`} />
                <div className="border-t border-cloud-elements-dividerColor my-2" />
                <SummaryRow label="Name" value={name} />
                <SummaryRow label="Image" value={image} mono />
                <SummaryRow label="Stack" value={stack} />
                <SummaryRow label="Resources" value={`${cpuCores} CPU · ${memoryMb} MB · ${diskGb} GB`} />
                <SummaryRow label="Max Lifetime" value={maxLifetime === 0 ? 'Unlimited' : `${maxLifetime}s`} />
                <SummaryRow label="Idle Timeout" value={`${idleTimeout}s`} />
                <SummaryRow label="SSH" value={sshEnabled ? 'Enabled' : 'Disabled'} />
                <SummaryRow label="Web Terminal" value={webTerminalEnabled ? 'Enabled' : 'Disabled'} />
              </div>

              {/* Pricing */}
              <div className="glass-card rounded-lg p-4">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm font-display font-medium text-cloud-elements-textSecondary">Estimated Cost</p>
                    <p className="text-xs text-cloud-elements-textTertiary mt-0.5">
                      {pricingTier?.multiplier ?? 50}x base rate
                    </p>
                  </div>
                  <Badge variant="accent">{pricingTier?.label ?? 'Create Sandbox'}</Badge>
                </div>
              </div>

              {/* Capacity info */}
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
                        {txStatus === 'confirmed' && 'Sandbox creation submitted!'}
                        {txStatus === 'failed' && 'Transaction failed'}
                      </p>
                      {txHash && (
                        <p className="text-xs font-data text-cloud-elements-textTertiary mt-0.5 truncate max-w-xs">
                          TX: {txHash}
                        </p>
                      )}
                      {txError && (
                        <p className="text-xs text-crimson-400 mt-0.5">{txError}</p>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {/* Provision Progress — visible after TX confirmed */}
              {txStatus === 'confirmed' && provisionCallId && (
                <ProvisionProgress
                  callId={provisionCallId}
                  onReady={(sandboxId) => {
                    updateSandboxStatus(name, 'running', { sidecarUrl: undefined });
                  }}
                />
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
                View Sandboxes
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
                    Deploy Sandbox
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
