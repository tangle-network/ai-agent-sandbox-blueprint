import { useParams, Link } from 'react-router';
import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Textarea } from '@tangle-network/blueprint-ui/components';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import { ResourceIdentity } from '~/components/shared/ResourceIdentity';
import { LabeledValueRow } from '~/components/shared/LabeledValueRow';
import { ExposedPortsCard } from '~/components/shared/ExposedPortsCard';
import { TeeAttestationCard } from '~/components/shared/TeeAttestationCard';
import { ResourceTabs } from '~/components/shared/ResourceTabs';
import { instanceListStore, getInstance, updateInstanceStatus } from '~/lib/stores/instances';
import { getBlueprint } from '@tangle-network/blueprint-ui';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { useOperatorApiCall } from '~/lib/hooks/useOperatorApiCall';
import { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { useTeeAttestation } from '~/lib/hooks/useTeeAttestation';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { useInstanceHydration } from '~/lib/hooks/useInstanceHydration';
import { createProxiedInstanceClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { cn } from '@tangle-network/blueprint-ui';
import { truncateAddress } from '@tangle-network/agent-ui/primitives';
import { OperatorTerminalView } from '~/components/shared/OperatorTerminalView';
import { ConfirmDialog } from '~/components/shared/ConfirmDialog';
import { useAccount } from 'wagmi';
import {
  getInstanceSandboxDisplayValue,
  getInstanceServiceDisplayValue,
  getInstanceStatusLabel,
} from '~/lib/instances/display';

type ActionTab = 'overview' | 'terminal' | 'chat' | 'secrets' | 'attestation';

/** Extract human-readable error from operator API Error messages. */
function parseApiError(err: Error): string {
  const idx = err.message.indexOf('): ');
  if (idx === -1) return err.message;
  const body = err.message.slice(idx + 3);
  try {
    const parsed = JSON.parse(body) as { error?: string };
    if (typeof parsed.error === 'string') return parsed.error;
  } catch {
    // ignore non-JSON error bodies
  }
  return err.message;
}

export default function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const instances = useStore(instanceListStore);
  const inst = getInstance(decodedId) ?? instances.find((s) => s.id === decodedId);
  const { address } = useAccount();

  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [secretsJson, setSecretsJson] = useState('{\n  \n}');
  const [secretsBusy, setSecretsBusy] = useState(false);
  const [secretsError, setSecretsError] = useState<string | null>(null);
  const [secretsSuccess, setSecretsSuccess] = useState<string | null>(null);
  const [confirmAction, setConfirmAction] = useState<{
    title: string;
    description: string;
    confirmLabel: string;
    onConfirm: () => void;
  } | null>(null);
  const timeoutsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set());

  const serviceId = inst?.serviceId ? BigInt(inst.serviceId) : null;
  const bpId = inst?.teeEnabled ? 'ai-agent-tee-instance-blueprint' : 'ai-agent-instance-blueprint';
  const isCreating = inst?.status === 'creating' && !inst?.sandboxId;

  // Watch for OperatorProvisioned event if instance is still creating
  const instanceProvision = useInstanceProvisionWatcher(
    serviceId,
    inst?.teeEnabled ? 'tee-instance' : 'instance',
    isCreating,
  );

  useEffect(() => {
    if (instanceProvision && decodedId) {
      updateInstanceStatus(decodedId, 'running', {
        sandboxId: instanceProvision.sandboxId,
        sidecarUrl: instanceProvision.sidecarUrl,
      });
    }
  }, [instanceProvision, decodedId]);

  useEffect(() => {
    return () => {
      for (const timeoutId of timeoutsRef.current) {
        clearTimeout(timeoutId);
      }
      timeoutsRef.current.clear();
    };
  }, []);

  const scheduleDismiss = useCallback((fn: () => void, ms: number) => {
    const timeoutId = setTimeout(() => {
      timeoutsRef.current.delete(timeoutId);
      fn();
    }, ms);
    timeoutsRef.current.add(timeoutId);
  }, []);

  // Operator API auth for browser access to interactive features and attestation.
  const operatorUrl = INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL;
  const {
    getToken: getOperatorToken,
    getCachedToken: getCachedOperatorToken,
    isAuthenticated: isOperatorAuthed,
    isAuthenticating: isOperatorAuthenticating,
    error: operatorAuthError,
  } = useOperatorAuth(operatorUrl);
  const buildPath = useCallback((action: string) => `/api/sandbox/${action}`, []);
  const operatorApiCall = useOperatorApiCall(operatorUrl, getOperatorToken, buildPath);
  const client: SandboxClient | null = useMemo(() => {
    return createProxiedInstanceClient(getOperatorToken, operatorUrl);
  }, [getOperatorToken, operatorUrl]);
  const operatorToken = getCachedOperatorToken();
  const hasWallet = !!address;
  const handleOperatorAuthenticate = useCallback(() => {
    void getOperatorToken();
  }, [getOperatorToken]);

  const ports = useExposedPorts(inst?.status, operatorApiCall);
  const { refresh: refreshInstances } = useInstanceHydration();

  const {
    attestation,
    busy: attestationBusy,
    error: attestationError,
    fetchAttestation: handleFetchAttestation,
  } = useTeeAttestation(operatorApiCall);

  const handleInjectSecrets = useCallback(async () => {
    setSecretsBusy(true);
    setSecretsError(null);
    setSecretsSuccess(null);
    try {
      const parsed = JSON.parse(secretsJson);
      if (typeof parsed !== 'object' || parsed == null || Array.isArray(parsed)) {
        throw new Error('Secrets must be a JSON object');
      }
      await operatorApiCall('secrets', { env_json: parsed });
      await refreshInstances({ interactive: true });
      setSecretsSuccess('Secrets injected');
      scheduleDismiss(() => setSecretsSuccess(null), 3000);
    } catch (e) {
      setSecretsError(e instanceof Error ? parseApiError(e) : 'Failed to inject secrets');
    } finally {
      setSecretsBusy(false);
    }
  }, [operatorApiCall, refreshInstances, scheduleDismiss, secretsJson]);

  const handleWipeSecrets = useCallback(() => {
    setConfirmAction({
      title: 'Wipe Secrets',
      description: 'This will remove all injected secrets and restart the instance without them.',
      confirmLabel: 'Wipe',
      onConfirm: () => {
        void (async () => {
          setSecretsBusy(true);
          setSecretsError(null);
          setSecretsSuccess(null);
          try {
            await operatorApiCall('secrets', undefined, { method: 'DELETE' });
            await refreshInstances({ interactive: true });
            setSecretsSuccess('Secrets wiped');
            scheduleDismiss(() => setSecretsSuccess(null), 3000);
          } catch (e) {
            setSecretsError(e instanceof Error ? parseApiError(e) : 'Failed to wipe secrets');
          } finally {
            setSecretsBusy(false);
          }
        })();
      },
    });
  }, [operatorApiCall, refreshInstances, scheduleDismiss]);

  if (!inst) {
    return (
      <AnimatedPage className="mx-auto max-w-3xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <div className="i-ph:cube text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
            <p className="text-cloud-elements-textSecondary">Instance not found</p>
            <Link to="/instances" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Instances</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  const hasAgent = !!inst.agentIdentifier;

  const tabs: { key: ActionTab; label: string; icon: string; hidden?: boolean }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal' },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle', hidden: !hasAgent },
    { key: 'secrets', label: 'Secrets', icon: 'i-ph:lock-simple', hidden: !!inst.teeEnabled },
    ...(inst.teeEnabled ? [{ key: 'attestation' as const, label: 'Attestation', icon: 'i-ph:shield-check' }] : []),
  ];

  return (
    <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
      {/* Breadcrumb */}
      <div className="flex items-center gap-2 mb-6 text-sm text-cloud-elements-textTertiary">
        <Link to="/instances" className="hover:text-cloud-elements-textSecondary transition-colors">Instances</Link>
        <span>/</span>
        <span className="text-cloud-elements-textPrimary font-display">{inst.name}</span>
      </div>

      {/* Header */}
      <div className="flex items-start mb-6">
        <div className="flex items-center gap-4">
          <div className={cn(
            'w-14 h-14 rounded-xl flex items-center justify-center',
            inst.status === 'running' ? 'bg-teal-500/10' : inst.status === 'creating' ? 'bg-violet-500/10' : 'bg-cloud-elements-background-depth-3',
          )}>
            <div className={cn(
              inst.teeEnabled ? 'i-ph:shield-check text-2xl' : 'i-ph:cube text-2xl',
              inst.status === 'running' ? 'text-teal-400' : inst.status === 'creating' ? 'text-violet-400' : 'text-cloud-elements-textTertiary',
            )} />
          </div>
          <ResourceIdentity
            name={inst.name}
            status={inst.status}
            statusLabel={getInstanceStatusLabel(inst)}
            teeEnabled={inst.teeEnabled}
            image={inst.image}
            specs={`${inst.cpuCores} CPU · ${inst.memoryMb}MB · ${inst.diskGb}GB`}
            titleClassName="text-xl"
            teeStyle="pill"
          />
        </div>
        {inst.status === 'running' && inst.serviceId && (
          <Link to={`/workflows?target=${encodeURIComponent(`instance:${inst.id}`)}`} className="ml-auto">
            <Button variant="secondary" size="sm">
              <div className="i-ph:flow-arrow text-sm" />
              Create Workflow
            </Button>
          </Link>
        )}
      </div>

      <ResourceTabs tabs={tabs} value={tab} onValueChange={setTab} className="mb-6" />

      {/* Overview */}
      {tab === 'overview' && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <Card>
            <CardHeader>
              <CardTitle>Instance Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <LabeledValueRow label="ID" value={inst.id} mono copyable />
              <LabeledValueRow
                label="Sandbox"
                value={getInstanceSandboxDisplayValue(inst)}
                mono={!!inst.sandboxId}
                copyable={!!inst.sandboxId}
                copyValue={inst.sandboxId ?? undefined}
              />
              <LabeledValueRow label="Image" value={inst.image} mono copyable />
              <LabeledValueRow label="CPU" value={`${inst.cpuCores} cores`} />
              <LabeledValueRow label="Memory" value={`${inst.memoryMb} MB`} />
              <LabeledValueRow label="Disk" value={`${inst.diskGb} GB`} />
              <LabeledValueRow label="Created" value={new Date(inst.createdAt).toLocaleString()} />
              <LabeledValueRow label="Blueprint" value={getBlueprint(bpId)?.name ?? bpId} />
              <LabeledValueRow label="Service" value={getInstanceServiceDisplayValue(inst)} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Runtime Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <LabeledValueRow
                label="Operator"
                value={inst.operator ? truncateAddress(inst.operator) : 'Unknown'}
                mono
                copyable={!!inst.operator}
                copyValue={inst.operator}
              />
              {inst.txHash && (
                <LabeledValueRow
                  label="TX Hash"
                  value={truncateAddress(inst.txHash)}
                  mono
                  copyable
                  copyValue={inst.txHash}
                />
              )}
            </CardContent>
          </Card>

          {/* Exposed Ports */}
          {ports && ports.length > 0 && (
            <ExposedPortsCard
              ports={ports}
              accessPath="/api/sandbox/port/{port}/"
              className="lg:col-span-2"
            />
          )}
        </div>
      )}

      {/* Terminal */}
      {tab === 'terminal' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0">
            {isOperatorAuthed && operatorToken ? (
              <div className="h-[min(500px,60vh)]">
                <OperatorTerminalView
                  apiUrl={operatorUrl}
                  resourcePath="/api/sandbox"
                  token={operatorToken}
                  title="Instance Terminal"
                  subtitle="Connected through the operator API"
                />
              </div>
            ) : (
              <div className="p-6 text-center">
                <p className="text-sm text-cloud-elements-textSecondary mb-3">
                  Authenticate with the operator to access the terminal
                </p>
                <p className="text-xs text-cloud-elements-textTertiary mb-4">
                  Commands are relayed through the operator API and no longer connect directly to the sandbox container.
                </p>
                {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
                <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
                  {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* Chat */}
      {tab === 'chat' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0">
            {isOperatorAuthed ? (
              <div className="h-[min(600px,65vh)]">
                <SessionSidebar
                  sandboxId={inst.sandboxId ?? decodedId}
                  client={client}
                  systemPrompt={systemPrompt}
                  onSystemPromptChange={setSystemPrompt}
                />
              </div>
            ) : (
              <div className="p-6 text-center">
                <p className="text-sm text-cloud-elements-textSecondary mb-3">
                  Authenticate with the operator to chat with the instance agent
                </p>
                <p className="text-xs text-cloud-elements-textTertiary mb-4">
                  Chat requests are proxied through the operator API and do not expose the sandbox container to the browser.
                </p>
                {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
                <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
                  {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* Secrets */}
      {tab === 'secrets' && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Environment Secrets</CardTitle>
              <CardDescription>Inject environment variables as secrets into the instance</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {isOperatorAuthed ? (
                <>
                  <div className="space-y-1.5">
                    <label className="text-xs font-medium text-cloud-elements-textSecondary" htmlFor="instance-secrets-json">
                      Secrets (JSON object)
                    </label>
                    <Textarea
                      id="instance-secrets-json"
                      value={secretsJson}
                      onChange={(e) => setSecretsJson(e.target.value)}
                      placeholder='{"API_KEY": "sk-...", "DB_URL": "postgres://..."}'
                      className="font-data text-xs min-h-[120px] resize-y"
                    />
                    <p className="text-[11px] text-cloud-elements-textTertiary">
                      Key-value pairs injected as environment variables. Values are stored securely and not readable after injection.
                    </p>
                  </div>
                  {secretsError && (
                    <p className="text-xs text-red-400">{secretsError}</p>
                  )}
                  {secretsSuccess && (
                    <p className="text-xs text-teal-400">{secretsSuccess}</p>
                  )}
                  <div className="flex items-center gap-2">
                    <Button size="sm" onClick={handleInjectSecrets} disabled={secretsBusy}>
                      {secretsBusy ? 'Injecting...' : 'Inject Secrets'}
                    </Button>
                    <Button variant="destructive" size="sm" onClick={handleWipeSecrets} disabled={secretsBusy}>
                      Wipe All Secrets
                    </Button>
                  </div>
                </>
              ) : (
                <div className="p-2 text-center">
                  <p className="text-sm text-cloud-elements-textSecondary mb-3">
                    Authenticate with the operator to manage instance secrets
                  </p>
                  <p className="text-xs text-cloud-elements-textTertiary mb-4">
                    Secret updates are proxied through the operator API and may restart the instance sidecar to apply changes.
                  </p>
                  {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
                  <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
                    {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
                  </Button>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}

      {/* Attestation Tab — TEE attestation verification */}
      {tab === 'attestation' && (
        <div className="space-y-4">
          <TeeAttestationCard
            subjectLabel="instance"
            attestation={attestation}
            busy={attestationBusy}
            error={attestationError}
            onFetch={handleFetchAttestation}
          />
        </div>
      )}

      <ConfirmDialog
        open={!!confirmAction}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title={confirmAction?.title ?? 'Confirm'}
        description={confirmAction?.description ?? ''}
        confirmLabel={confirmAction?.confirmLabel}
        onConfirm={() => {
          confirmAction?.onConfirm();
        }}
        variant="danger"
      />
    </AnimatedPage>
  );
}
