import { useParams, Link } from 'react-router';
import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Input, Textarea } from '@tangle-network/blueprint-ui/components';
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
import { SnapshotDialog } from '~/components/shared/SnapshotDialog';

import { useAccount } from 'wagmi';
import {
  getInstanceSandboxDisplayValue,
  getInstanceServiceDisplayValue,
  getInstanceStatusLabel,
} from '~/lib/instances/display';

interface SshKey {
  username: string;
  publicKey: string;
}

type ActionTab = 'overview' | 'terminal' | 'chat' | 'ssh' | 'secrets' | 'attestation';

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
  const isRunning = inst?.status === 'running';

  const sshHost = useMemo(() => {
    if (!inst?.sidecarUrl) return '';
    try {
      return new URL(inst.sidecarUrl).hostname;
    } catch {
      return '';
    }
  }, [inst?.sidecarUrl]);

  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [secretsJson, setSecretsJson] = useState('{\n  \n}');
  const [secretsBusy, setSecretsBusy] = useState(false);
  const [secretsError, setSecretsError] = useState<string | null>(null);
  const [secretsSuccess, setSecretsSuccess] = useState<string | null>(null);
  const [secretsLoading, setSecretsLoading] = useState(false);
  const secretsFetchedRef = useRef(false);
  // SSH state
  const [sshPublicKey, setSshPublicKey] = useState('');
  const [sshUsername, setSshUsername] = useState('');
  const [sshKeys, setSshKeys] = useState<SshKey[]>([]);
  const [sshBusy, setSshBusy] = useState(false);
  const [sshError, setSshError] = useState<string | null>(null);
  const [sshSuccess, setSshSuccess] = useState<string | null>(null);
  const [sshUserDetecting, setSshUserDetecting] = useState(false);
  const [sshUserHint, setSshUserHint] = useState<string | null>(null);
  const sshUsernameDirtyRef = useRef(false);
  const sshUserDetectionKeyRef = useRef<string | null>(null);

  const [snapshotOpen, setSnapshotOpen] = useState(false);
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

  const handleSnapshot = useCallback(
    async (params: { destination: string; include_workspace: boolean; include_state: boolean }) => {
      await operatorApiCall('snapshot', params);
    },
    [operatorApiCall],
  );

  const sshDetectionKey = inst?.sandboxId ?? decodedId;
  const sshConnectionCommand = useMemo(() => {
    if (!inst?.sshPort || !sshHost) return '';
    const user = sshUsername.trim() || 'sidecar';
    return `ssh ${user}@${sshHost} -p ${inst.sshPort}`;
  }, [inst?.sshPort, sshHost, sshUsername]);
  const handleOperatorAuthenticate = useCallback(() => {
    void getOperatorToken();
  }, [getOperatorToken]);

  // Reset SSH username state when switching between instances
  useEffect(() => {
    sshUsernameDirtyRef.current = false;
    sshUserDetectionKeyRef.current = null;
    setSshUsername('');
    setSshUserHint(null);
    setSshUserDetecting(false);
  }, [sshDetectionKey]);

  // Auto-detect SSH username when SSH tab opens
  useEffect(() => {
    if (tab !== 'ssh' || !sshDetectionKey) return;
    if (!isRunning) return;
    if (!isOperatorAuthed && !operatorToken) return;
    if (sshUserDetectionKeyRef.current === sshDetectionKey) return;

    let cancelled = false;
    sshUserDetectionKeyRef.current = sshDetectionKey;
    setSshUserDetecting(true);
    setSshUserHint(null);

    void operatorApiCall('ssh/user', undefined, { method: 'GET' })
      .then(async (response) => {
        const body = await response.json() as { username?: string };
        const detectedUsername = typeof body.username === 'string' ? body.username.trim() : '';
        if (cancelled) return;
        if (detectedUsername) {
          if (!sshUsernameDirtyRef.current) {
            setSshUsername(detectedUsername);
          }
          setSshUserHint(`Detected sandbox user: ${detectedUsername}`);
        } else {
          setSshUserHint('Could not detect the sandbox user. You can enter one manually.');
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setSshUserHint(
          e instanceof Error
            ? `Could not detect the sandbox user: ${parseApiError(e)}`
            : 'Could not detect the sandbox user. You can enter one manually.',
        );
      })
      .finally(() => {
        if (!cancelled) {
          setSshUserDetecting(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [tab, sshDetectionKey, isRunning, isOperatorAuthed, operatorToken, operatorApiCall]);

  // Fetch existing secrets when secrets tab becomes active
  useEffect(() => {
    if (tab !== 'secrets') return;
    if (!isRunning) return;
    if (!isOperatorAuthed && !operatorToken) return;
    if (secretsFetchedRef.current) return;

    let cancelled = false;
    secretsFetchedRef.current = true;
    setSecretsLoading(true);

    void operatorApiCall('secrets', undefined, { method: 'GET' })
      .then(async (response) => {
        const body = (await response.json()) as { env_json?: Record<string, unknown> };
        if (cancelled) return;
        if (body.env_json && Object.keys(body.env_json).length > 0) {
          setSecretsJson(JSON.stringify(body.env_json, null, 2));
        }
      })
      .catch(() => {
        // Non-fatal: user sees empty editor as fallback
      })
      .finally(() => {
        if (!cancelled) setSecretsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [tab, isRunning, isOperatorAuthed, operatorToken, operatorApiCall]);

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
            setSecretsJson('{\n  \n}');
            secretsFetchedRef.current = false;
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

  const handleSshProvision = useCallback(async () => {
    const key = sshPublicKey.trim();
    if (!key) return;

    const requestedUsername = sshUsername.trim();

    if (requestedUsername && requestedUsername.length > 32) {
      setSshError('Username must be 32 characters or fewer');
      return;
    }
    if (requestedUsername && !/^[a-zA-Z0-9\-_.]+$/.test(requestedUsername)) {
      setSshError('Username may only contain letters, numbers, dashes, underscores, and dots');
      return;
    }

    const validPrefixes = ['ssh-rsa ', 'ssh-ed25519 ', 'ssh-dss ', 'ecdsa-sha2-'];
    if (!validPrefixes.some((p) => key.startsWith(p))) {
      setSshError('Invalid SSH key format. Must start with ssh-rsa, ssh-ed25519, or ecdsa-sha2-*');
      return;
    }
    setSshBusy(true);
    setSshError(null);
    setSshSuccess(null);
    try {
      const payload: Record<string, unknown> = { public_key: key };
      if (requestedUsername) {
        payload.username = requestedUsername;
      }
      const response = await operatorApiCall('ssh', payload);
      const body = await response.json() as { username?: string };
      const effectiveUsername = typeof body.username === 'string' && body.username.trim()
        ? body.username.trim()
        : requestedUsername;
      setSshKeys((prev) => {
        const exists = prev.some((k) => k.publicKey === key);
        if (exists) {
          return prev.map((k) => k.publicKey === key ? { ...k, username: effectiveUsername } : k);
        }
        return [...prev, { username: effectiveUsername, publicKey: key }];
      });
      sshUsernameDirtyRef.current = false;
      setSshUsername(effectiveUsername);
      setSshUserHint(`Detected sandbox user: ${effectiveUsername}`);
      setSshPublicKey('');
      setSshSuccess('SSH key provisioned');
      scheduleDismiss(() => setSshSuccess(null), 3000);
    } catch (e) {
      setSshError(e instanceof Error ? parseApiError(e) : 'Failed to provision SSH key');
    } finally {
      setSshBusy(false);
    }
  }, [sshUsername, sshPublicKey, operatorApiCall, scheduleDismiss]);

  const handleSshRevoke = useCallback(async (key: SshKey) => {
    setSshBusy(true);
    setSshError(null);
    setSshSuccess(null);
    try {
      const response = await operatorApiCall(
        'ssh',
        { username: key.username, public_key: key.publicKey },
        { method: 'DELETE' },
      );
      const body = await response.json() as { username?: string };
      const effectiveUsername = typeof body.username === 'string' && body.username.trim()
        ? body.username.trim()
        : key.username;
      setSshKeys((prev) => prev.filter((k) => k.publicKey !== key.publicKey));
      sshUsernameDirtyRef.current = false;
      setSshUsername(effectiveUsername);
      setSshUserHint(`Detected sandbox user: ${effectiveUsername}`);
      setSshSuccess('SSH key revoked');
      scheduleDismiss(() => setSshSuccess(null), 3000);
    } catch (e) {
      setSshError(e instanceof Error ? parseApiError(e) : 'Failed to revoke SSH key');
    } finally {
      setSshBusy(false);
    }
  }, [operatorApiCall, scheduleDismiss]);

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
    { key: 'ssh' as const, label: 'SSH', icon: 'i-ph:key', hidden: !inst.sshPort },
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
        {inst.status === 'running' && (
          <div className="ml-auto flex items-center gap-2">
            <Button variant="secondary" size="sm" onClick={() => setSnapshotOpen(true)}>
              <div className="i-ph:camera text-sm" />
              Snapshot
            </Button>
            {inst.serviceId && (
              <Link to={`/workflows?target=${encodeURIComponent(`instance:${inst.id}`)}`}>
                <Button variant="secondary" size="sm">
                  <div className="i-ph:flow-arrow text-sm" />
                  Create Workflow
                </Button>
              </Link>
            )}
          </div>
        )}
      </div>

      {inst.circuitBreakerActive && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Sidecar unreachable — circuit breaker active
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            {inst.circuitBreakerProbing
              ? 'Recovery probe in progress\u2026'
              : `Cooldown active — retrying in ~${inst.circuitBreakerRemainingSecs ?? '?'}s`}
          </p>
        </div>
      )}

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
              proxyBaseUrl={`${operatorUrl}/api/sandbox/port/`}
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
          {!isOperatorAuthed ? (
            <CardContent className="p-0">
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
            </CardContent>
          ) : inst.credentialsAvailable === false ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:key text-3xl text-amber-400 mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                AI credentials are not configured
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-3">
                Add one of the following in the Secrets tab:
              </p>
              <ul className="text-xs text-cloud-elements-textTertiary space-y-1 mb-4">
                <li><code className="font-data">ANTHROPIC_API_KEY</code></li>
                <li><code className="font-data">ZAI_API_KEY</code></li>
                <li><code className="font-data">OPENCODE_MODEL_PROVIDER</code> + <code className="font-data">OPENCODE_MODEL_NAME</code> + <code className="font-data">OPENCODE_MODEL_API_KEY</code></li>
              </ul>
              <Button size="sm" variant="outline" onClick={() => setTab('secrets')}>
                Go to Secrets
              </Button>
            </CardContent>
          ) : (
            <CardContent className="p-0">
              <div className="h-[min(600px,65vh)]">
                <SessionSidebar
                  sandboxId={inst.sandboxId ?? decodedId}
                  client={client}
                  systemPrompt={systemPrompt}
                  onSystemPromptChange={setSystemPrompt}
                />
              </div>
            </CardContent>
          )}
        </Card>
      )}

      {/* SSH Tab — provision and revoke SSH keys */}
      {tab === 'ssh' && (
        <div className="space-y-4">
          {sshConnectionCommand && (
            <Card>
              <CardHeader>
                <CardTitle className="text-sm">SSH Connection</CardTitle>
                <CardDescription>
                  Connect to this instance via SSH using the command below.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-2">
                <p className="text-xs font-data rounded-lg bg-cloud-elements-background-depth-2 px-3 py-2 break-all">
                  {sshConnectionCommand}
                </p>
              </CardContent>
            </Card>
          )}

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Add SSH Key</CardTitle>
              <CardDescription>Provision an SSH public key for remote access</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-cloud-elements-textSecondary">Username</label>
                <Input
                  aria-label="SSH username"
                  value={sshUsername}
                  onChange={(e) => {
                    sshUsernameDirtyRef.current = true;
                    setSshUsername(e.target.value);
                  }}
                  placeholder={sshUserDetecting ? 'Detecting sandbox user...' : 'Auto-detected from sandbox'}
                  className="font-data text-sm"
                />
                {sshUserHint && (
                  <p className="text-xs text-cloud-elements-textSecondary">{sshUserHint}</p>
                )}
              </div>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-cloud-elements-textSecondary">Public Key</label>
                <Textarea
                  aria-label="SSH public key"
                  value={sshPublicKey}
                  onChange={(e) => setSshPublicKey(e.target.value)}
                  placeholder="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5..."
                  className="font-data text-xs min-h-[80px] resize-none"
                />
              </div>
              {sshError && (
                <p className="text-xs text-red-400">{sshError}</p>
              )}
              {sshSuccess && (
                <p className="text-xs text-teal-400">{sshSuccess}</p>
              )}
              <Button
                size="sm"
                onClick={handleSshProvision}
                disabled={sshBusy || !sshPublicKey.trim()}
              >
                {sshBusy ? 'Provisioning...' : 'Add Key'}
              </Button>
            </CardContent>
          </Card>

          {sshKeys.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className="text-sm">Active Keys</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {sshKeys.map((key) => (
                  <div
                    key={key.publicKey}
                    className="flex items-center justify-between gap-3 p-3 rounded-lg bg-cloud-elements-background-depth-2"
                  >
                    <div className="min-w-0">
                      <span className="text-xs font-data text-cloud-elements-textSecondary">{key.username}@</span>
                      <span className="text-xs font-data text-cloud-elements-textTertiary truncate block">
                        {key.publicKey.length > 60 ? `${key.publicKey.slice(0, 60)}...` : key.publicKey}
                      </span>
                    </div>
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => handleSshRevoke(key)}
                      disabled={sshBusy}
                    >
                      Revoke
                    </Button>
                  </div>
                ))}
              </CardContent>
            </Card>
          )}
        </div>
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
                    {secretsLoading && (
                      <p className="text-xs text-cloud-elements-textTertiary">Loading existing secrets...</p>
                    )}
                    <Textarea
                      id="instance-secrets-json"
                      value={secretsJson}
                      onChange={(e) => setSecretsJson(e.target.value)}
                      placeholder='{"API_KEY": "sk-...", "DB_URL": "postgres://..."}'
                      className="font-data text-xs min-h-[120px] resize-y"
                      disabled={secretsLoading}
                    />
                    <p className="text-[11px] text-cloud-elements-textTertiary">
                      Key-value pairs injected as environment variables. Injecting replaces all existing secrets. Values are encrypted at rest.
                    </p>
                  </div>
                  {secretsError && (
                    <p className="text-xs text-red-400">{secretsError}</p>
                  )}
                  {secretsSuccess && (
                    <p className="text-xs text-teal-400">{secretsSuccess}</p>
                  )}
                  <div className="flex items-center gap-2">
                    <Button size="sm" onClick={handleInjectSecrets} disabled={secretsBusy || secretsLoading}>
                      {secretsBusy ? 'Injecting...' : 'Inject Secrets'}
                    </Button>
                    <Button variant="destructive" size="sm" onClick={handleWipeSecrets} disabled={secretsBusy || secretsLoading}>
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

      <SnapshotDialog
        open={snapshotOpen}
        onOpenChange={setSnapshotOpen}
        onConfirm={handleSnapshot}
      />
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
