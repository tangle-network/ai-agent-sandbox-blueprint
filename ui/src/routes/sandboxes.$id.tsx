import { useParams, Link, useNavigate } from 'react-router';
import { lazy, Suspense, useState, useCallback, useMemo, useRef, useEffect } from 'react';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Textarea } from '@tangle-network/blueprint-ui/components';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import { ResourceIdentity } from '~/components/shared/ResourceIdentity';
import { LabeledValueRow } from '~/components/shared/LabeledValueRow';
import { ExposedPortsCard } from '~/components/shared/ExposedPortsCard';
import { TeeAttestationCard } from '~/components/shared/TeeAttestationCard';
import { ResourceTabs } from '~/components/shared/ResourceTabs';
import {
  sandboxListStore,
  findSandboxByKey,
  getSandboxRouteKey,
  updateSandboxStatus,
} from '~/lib/stores/sandboxes';
import { useSandboxActive, useSandboxOperator } from '~/lib/hooks/useSandboxReads';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { useSubmitJob } from '@tangle-network/blueprint-ui';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { useWagmiSidecarAuth } from '~/lib/hooks/useWagmiSidecarAuth';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { useOperatorApiCall } from '~/lib/hooks/useOperatorApiCall';
import { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { useTeeAttestation } from '~/lib/hooks/useTeeAttestation';
import { createDirectClient, createProxiedClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { cn } from '@tangle-network/blueprint-ui';
import { ConfirmDialog } from '~/components/shared/ConfirmDialog';

const TerminalView = lazy(() =>
  import('@tangle-network/agent-ui/terminal').then((m) => ({ default: m.TerminalView }))
);

type ActionTab = 'overview' | 'terminal' | 'chat' | 'ssh' | 'secrets' | 'attestation';

import { OPERATOR_API_URL, INSTANCE_OPERATOR_API_URL } from '~/lib/config';

interface SshKey {
  username: string;
  publicKey: string;
}

export default function SandboxDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const decodedKey = id ? decodeURIComponent(id) : '';
  const sandboxes = useStore(sandboxListStore);
  const sb = findSandboxByKey(sandboxes, decodedKey);
  const canonicalSandboxId = sb?.sandboxId;
  const routeKey = sb ? getSandboxRouteKey(sb) : decodedKey;

  const { data: isActive } = useSandboxActive(canonicalSandboxId);
  const { data: operator } = useSandboxOperator(canonicalSandboxId);
  const { submitJob, status: txStatus, txHash } = useSubmitJob();

  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');

  // SSH state
  const [sshPublicKey, setSshPublicKey] = useState('');
  const [sshUsername, setSshUsername] = useState('agent');
  const [sshKeys, setSshKeys] = useState<SshKey[]>([]);
  const [sshBusy, setSshBusy] = useState(false);
  const [sshError, setSshError] = useState<string | null>(null);
  const [sshSuccess, setSshSuccess] = useState<string | null>(null);

  // Secrets state
  const [secretsJson, setSecretsJson] = useState('{\n  \n}');
  const [secretsBusy, setSecretsBusy] = useState(false);
  const [secretsError, setSecretsError] = useState<string | null>(null);
  const [secretsSuccess, setSecretsSuccess] = useState<string | null>(null);

  // Confirm dialog state
  const [confirmAction, setConfirmAction] = useState<{ title: string; description: string; confirmLabel: string; onConfirm: () => void } | null>(null);

  // Track setTimeout IDs so they can be cleared on unmount
  const timeoutsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set());
  useEffect(() => {
    return () => {
      for (const id of timeoutsRef.current) {
        clearTimeout(id);
      }
      timeoutsRef.current.clear();
    };
  }, []);

  /** Schedule a timeout and track it for cleanup on unmount. */
  const scheduleDismiss = useCallback((fn: () => void, ms: number) => {
    const id = setTimeout(() => {
      timeoutsRef.current.delete(id);
      fn();
    }, ms);
    timeoutsRef.current.add(id);
  }, []);

  const serviceId = BigInt(sb?.serviceId ?? '1');

  // Resolve correct operator API URL (instance blueprints run on a different port)
  const instanceBpId = import.meta.env.VITE_INSTANCE_BLUEPRINT_ID;
  const teeBpId = import.meta.env.VITE_TEE_INSTANCE_BLUEPRINT_ID;
  const isInstance = sb ? (sb.blueprintId === instanceBpId || sb.blueprintId === teeBpId) : false;
  const operatorUrl = isInstance ? (INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL) : OPERATOR_API_URL;

  // Sidecar auth for PTY terminal and chat (direct connection)
  const sidecarUrl = sb?.sidecarUrl ?? '';
  const {
    token: sidecarToken,
    isAuthenticated: isSidecarAuthed,
    authenticate: sidecarAuth,
    isAuthenticating,
  } = useWagmiSidecarAuth(canonicalSandboxId ?? '', sidecarUrl);

  // Operator API auth for lifecycle operations (stop/resume/snapshot/ssh/secrets)
  const { getToken: getOperatorToken } = useOperatorAuth(operatorUrl);
  const buildPath = useCallback(
    (action: string) =>
      isInstance
        ? `/api/sandbox/${action}`
        : `/api/sandboxes/${encodeURIComponent(canonicalSandboxId ?? '__draft__')}/${action}`,
    [canonicalSandboxId, isInstance],
  );
  const operatorApiCall = useOperatorApiCall(operatorUrl, getOperatorToken, buildPath);
  const ports = useExposedPorts(canonicalSandboxId ? sb?.status : undefined, operatorApiCall);

  // Chat client: direct sidecar mode when authed, otherwise proxied operator mode.
  const client: SandboxClient | null = useMemo(() => {
    if (sb?.sidecarUrl && sidecarToken) {
      return createDirectClient(sb.sidecarUrl, sidecarToken);
    }
    if (!canonicalSandboxId) return null;
    return createProxiedClient(canonicalSandboxId, getOperatorToken, operatorUrl);
  }, [sb?.sidecarUrl, sidecarToken, canonicalSandboxId, getOperatorToken, operatorUrl]);

  useEffect(() => {
    if (!sb?.sandboxId) return;
    if (decodedKey === sb.sandboxId) return;
    navigate(`/sandboxes/${encodeURIComponent(sb.sandboxId)}`, { replace: true });
  }, [sb?.sandboxId, decodedKey, navigate]);

  const bpId = 'ai-agent-sandbox-blueprint';

  /** Compute job value from pricing tier (base rate = 0.001 TNT = 1e15 wei) */
  const jobValue = (jobId: number): bigint =>
    BigInt(PRICING_TIERS[jobId]?.multiplier ?? 1) * 1_000_000_000_000_000n;

  const encodeCtxJob = useCallback(
    (jobId: number, ctx: Record<string, unknown>, formValues: Record<string, unknown> = {}) => {
      const job = getJobById(bpId, jobId);
      if (!job) throw new Error(`Job ${jobId} not found`);
      return encodeJobArgs(job, formValues, ctx);
    },
    [],
  );

  const handleStop = useCallback(async () => {
    if (!canonicalSandboxId) return;
    try {
      await operatorApiCall('stop');
      updateSandboxStatus(routeKey, 'stopped');
    } catch (e) {
      console.error('Stop failed:', e);
      toast.error('Failed to stop sandbox');
    }
  }, [canonicalSandboxId, operatorApiCall, routeKey]);

  const handleResume = useCallback(async () => {
    if (!canonicalSandboxId) return;
    try {
      await operatorApiCall('resume');
      updateSandboxStatus(routeKey, 'running');
    } catch (e) {
      console.error('Resume failed:', e);
      toast.error('Failed to resume sandbox');
    }
  }, [canonicalSandboxId, operatorApiCall, routeKey]);

  const handleDelete = useCallback(() => {
    setConfirmAction({
      title: 'Delete Sandbox',
      description: 'This action is irreversible and will submit an on-chain transaction. All sandbox data will be permanently deleted.',
      confirmLabel: 'Delete',
      onConfirm: async () => {
        if (!canonicalSandboxId) return;
        const hash = await submitJob({
          serviceId,
          jobId: JOB_IDS.SANDBOX_DELETE,
          args: encodeCtxJob(JOB_IDS.SANDBOX_DELETE, { sandbox_id: canonicalSandboxId }),
          label: `Delete: ${canonicalSandboxId}`,
          value: jobValue(JOB_IDS.SANDBOX_DELETE),
        });
        if (hash) updateSandboxStatus(routeKey, 'gone');
      },
    });
  }, [canonicalSandboxId, routeKey, serviceId, submitJob, encodeCtxJob]);

  const handleSnapshot = useCallback(async () => {
    if (!canonicalSandboxId) return;
    try {
      await operatorApiCall('snapshot', {
        destination: '',
        include_workspace: true,
        include_state: true,
      });
      toast.success('Snapshot created');
    } catch (e) {
      console.error('Snapshot failed:', e);
      toast.error('Failed to snapshot sandbox');
    }
  }, [canonicalSandboxId, operatorApiCall]);

  // SSH handlers
  const handleSshProvision = useCallback(async () => {
    const key = sshPublicKey.trim();
    if (!key) return;
    const validPrefixes = ['ssh-rsa ', 'ssh-ed25519 ', 'ssh-dss ', 'ecdsa-sha2-'];
    if (!validPrefixes.some((p) => key.startsWith(p))) {
      setSshError('Invalid SSH key format. Must start with ssh-rsa, ssh-ed25519, or ecdsa-sha2-*');
      return;
    }
    setSshBusy(true);
    setSshError(null);
    setSshSuccess(null);
    try {
      await operatorApiCall('ssh', { username: sshUsername, public_key: key });
      setSshKeys((prev) => [...prev, { username: sshUsername, publicKey: sshPublicKey.trim() }]);
      setSshPublicKey('');
      setSshSuccess('SSH key provisioned');
      scheduleDismiss(() => setSshSuccess(null), 3000);
    } catch (e) {
      setSshError(e instanceof Error ? e.message : 'Failed to provision SSH key');
    } finally {
      setSshBusy(false);
    }
  }, [sshUsername, sshPublicKey, operatorApiCall, scheduleDismiss]);

  const handleSshRevoke = useCallback(async (key: SshKey) => {
    setSshBusy(true);
    setSshError(null);
    setSshSuccess(null);
    try {
      await operatorApiCall('ssh', { username: key.username, public_key: key.publicKey }, { method: 'DELETE' });
      setSshKeys((prev) => prev.filter((k) => k.publicKey !== key.publicKey));
      setSshSuccess('SSH key revoked');
      scheduleDismiss(() => setSshSuccess(null), 3000);
    } catch (e) {
      setSshError(e instanceof Error ? e.message : 'Failed to revoke SSH key');
    } finally {
      setSshBusy(false);
    }
  }, [operatorApiCall, scheduleDismiss]);

  // Secrets handlers
  const handleInjectSecrets = useCallback(async () => {
    setSecretsBusy(true);
    setSecretsError(null);
    setSecretsSuccess(null);
    try {
      const parsed = JSON.parse(secretsJson);
      if (typeof parsed !== 'object' || Array.isArray(parsed)) {
        throw new Error('Secrets must be a JSON object');
      }
      await operatorApiCall('secrets', { env_json: parsed });
      setSecretsSuccess('Secrets injected');
      scheduleDismiss(() => setSecretsSuccess(null), 3000);
    } catch (e) {
      setSecretsError(e instanceof Error ? e.message : 'Failed to inject secrets');
    } finally {
      setSecretsBusy(false);
    }
  }, [secretsJson, operatorApiCall, scheduleDismiss]);

  const handleWipeSecrets = useCallback(() => {
    setConfirmAction({
      title: 'Wipe Secrets',
      description: 'This will remove all injected secrets and restart the sandbox without them.',
      confirmLabel: 'Wipe',
      onConfirm: async () => {
        setSecretsBusy(true);
        setSecretsError(null);
        setSecretsSuccess(null);
        try {
          await operatorApiCall('secrets', undefined, { method: 'DELETE' });
          setSecretsSuccess('Secrets wiped');
          scheduleDismiss(() => setSecretsSuccess(null), 3000);
        } catch (e) {
          setSecretsError(e instanceof Error ? e.message : 'Failed to wipe secrets');
        } finally {
          setSecretsBusy(false);
        }
      },
    });
  }, [operatorApiCall, scheduleDismiss]);

  const {
    attestation,
    busy: attestationBusy,
    error: attestationError,
    fetchAttestation: handleFetchAttestation,
  } = useTeeAttestation(operatorApiCall);

  if (!sb) {
    return (
      <AnimatedPage className="mx-auto max-w-3xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center py-16">
            <div className="i-ph:hard-drives text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
            <p className="text-cloud-elements-textSecondary font-display">Sandbox not found</p>
            <Link to="/sandboxes" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Sandboxes</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  const isCreating = sb.status === 'creating';
  const hasProvisionedSandbox = !!canonicalSandboxId;
  const isRunning = sb.status === 'running';
  const isStopped = sb.status === 'stopped' || sb.status === 'warm';
  const isGone = sb.status === 'gone';

  const hasAgent = !!sb.agentIdentifier;

  const tabs: { key: ActionTab; label: string; icon: string; disabled?: boolean; hidden?: boolean }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle', disabled: !hasProvisionedSandbox || !isRunning, hidden: !hasAgent },
    { key: 'ssh', label: 'SSH', icon: 'i-ph:key', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'secrets', label: 'Secrets', icon: 'i-ph:lock-simple', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'attestation', label: 'Attestation', icon: 'i-ph:shield-check', disabled: !hasProvisionedSandbox || !sb.teeEnabled },
  ];

  return (
    <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
      {/* Header */}
      <div className="flex items-center gap-2 mb-6 text-sm text-cloud-elements-textTertiary">
        <Link to="/sandboxes" className="hover:text-cloud-elements-textSecondary transition-colors">Sandboxes</Link>
        <span>/</span>
        <span className="text-cloud-elements-textPrimary font-display">{sb.name}</span>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div className="flex items-center gap-4">
          <div className={cn(
            'w-14 h-14 rounded-xl flex items-center justify-center',
            isRunning ? 'bg-teal-500/10' : isStopped ? 'bg-amber-500/10' : 'bg-cloud-elements-background-depth-3',
          )}>
            <div className={cn(
              sb.teeEnabled ? 'i-ph:shield-check text-2xl' : 'i-ph:hard-drives text-2xl',
              isRunning ? 'text-teal-400' : isStopped ? 'text-amber-400' : 'text-cloud-elements-textTertiary',
            )} />
          </div>
          <ResourceIdentity
            name={sb.name}
            status={sb.status}
            teeEnabled={sb.teeEnabled}
            image={sb.image}
            specs={`${sb.cpuCores} CPU · ${sb.memoryMb}MB · ${sb.diskGb}GB`}
            titleClassName="text-xl"
            teeStyle="pill"
          />
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          {hasProvisionedSandbox && isRunning && !isCreating && (
            <Button variant="secondary" size="sm" onClick={handleStop}>
              <div className="i-ph:stop text-sm" />
              Stop
            </Button>
          )}
          {hasProvisionedSandbox && isStopped && (
            <Button variant="success" size="sm" onClick={handleResume}>
              <div className="i-ph:play text-sm" />
              Resume
            </Button>
          )}
          {hasProvisionedSandbox && !isGone && (
            <>
              <Button variant="secondary" size="sm" onClick={handleSnapshot}>
                <div className="i-ph:camera text-sm" />
                Snapshot
              </Button>
              <Button variant="destructive" size="sm" onClick={handleDelete}>
                <div className="i-ph:trash text-sm" />
                Delete
                <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_DELETE} pricingMultiplier={PRICING_TIERS[JOB_IDS.SANDBOX_DELETE]?.multiplier ?? 1} compact />
              </Button>
            </>
          )}
        </div>
      </div>

      <ResourceTabs tabs={tabs} value={tab} onValueChange={setTab} className="mb-6" />

      {/* Provision Progress (shown when creating) */}
      {sb.status === 'creating' && sb.callId && (
        <ProvisionProgress
          callId={sb.callId}
          className="mb-4"
          onReady={(sandboxId, sidecarUrl) => {
            updateSandboxStatus(routeKey, 'running', { sandboxId, sidecarUrl });
          }}
          onFailed={() => updateSandboxStatus(routeKey, 'error')}
        />
      )}

      {/* Tab Content */}
      {tab === 'overview' && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Configuration</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2.5">
              <LabeledValueRow
                label="Sandbox ID"
                value={sb.sandboxId || 'Pending operator provision'}
                mono={!!sb.sandboxId}
                copyable={!!sb.sandboxId}
                alignRight
              />
              {sb.sandboxId == null && (
                <LabeledValueRow label="Draft Key" value={sb.localId} mono alignRight />
              )}
              <LabeledValueRow label="Image" value={sb.image} mono copyable alignRight />
              <LabeledValueRow label="CPU" value={`${sb.cpuCores} cores`} alignRight />
              <LabeledValueRow label="Memory" value={`${sb.memoryMb} MB`} alignRight />
              <LabeledValueRow label="Disk" value={`${sb.diskGb} GB`} alignRight />
              <LabeledValueRow label="Created" value={new Date(sb.createdAt).toLocaleString()} alignRight />
              <LabeledValueRow label="Blueprint" value={sb.blueprintId} mono alignRight />
              <LabeledValueRow label="Service ID" value={sb.serviceId} alignRight />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">On-Chain Status</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2.5">
              <LabeledValueRow label="Active" value={isActive !== undefined ? (isActive ? 'Yes' : 'No') : 'Loading...'} alignRight />
              <LabeledValueRow
                label="Operator"
                value={operator && operator !== '0x0000000000000000000000000000000000000000' ? operator : 'Unassigned'}
                mono
                alignRight
              />
              {sb.txHash && <LabeledValueRow label="TX Hash" value={sb.txHash} mono copyable alignRight />}
              {sb.sidecarUrl ? (
                <LabeledValueRow label="Sidecar" value={sb.sidecarUrl} mono copyable alignRight />
              ) : sb.status === 'creating' ? (
                <div className="flex justify-between items-center text-sm">
                  <span className="text-cloud-elements-textSecondary">Sidecar</span>
                  <span className="flex items-center gap-2 text-xs font-data text-violet-400">
                    <div className="i-ph:circle-fill text-[8px] animate-pulse" />
                    Provisioning...
                  </span>
                </div>
              ) : null}
            </CardContent>
          </Card>

          {/* Exposed Ports */}
          {ports && ports.length > 0 && (
            <ExposedPortsCard
              ports={ports}
              accessPath="/api/sandboxes/{id}/port/{port}/"
              className="md:col-span-2"
            />
          )}
        </div>
      )}

      {/* Terminal Tab — real PTY via sidecar */}
      {tab === 'terminal' && (
        <Card className="overflow-hidden">
          {!isSidecarAuthed ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:terminal-window text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                Authenticate to access the sandbox terminal
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-4">
                You'll be asked to sign a message with your wallet to verify ownership
              </p>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => sidecarAuth()}
                disabled={isAuthenticating || !sidecarUrl}
              >
                {isAuthenticating ? 'Signing...' : !sidecarUrl ? 'Waiting for sidecar...' : 'Connect Terminal'}
              </Button>
            </CardContent>
          ) : (
            <CardContent className="p-0">
              <div className="h-[min(500px,60vh)]">
                <Suspense fallback={
                  <div className="flex items-center justify-center h-full bg-neutral-950">
                    <span className="text-sm text-neutral-500">Loading terminal...</span>
                  </div>
                }>
                  <TerminalView
                    apiUrl={sidecarUrl}
                    token={sidecarToken!}
                    title="Sandbox Terminal"
                    subtitle="Connected to sidecar PTY session"
                  />
                </Suspense>
              </div>
            </CardContent>
          )}
        </Card>
      )}

      {/* Chat Tab — multi-session agent chat */}
      {tab === 'chat' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0">
            <div className="h-[min(600px,65vh)]">
              <SessionSidebar
                sandboxId={canonicalSandboxId ?? sb.localId}
                client={client}
                systemPrompt={systemPrompt}
                onSystemPromptChange={setSystemPrompt}
              />
            </div>
          </CardContent>
        </Card>
      )}

      {/* SSH Tab — provision and revoke SSH keys */}
      {tab === 'ssh' && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Add SSH Key</CardTitle>
              <CardDescription>Provision an SSH public key for remote access</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-cloud-elements-textSecondary">Username</label>
                <Input
                  value={sshUsername}
                  onChange={(e) => setSshUsername(e.target.value)}
                  placeholder="agent"
                  className="font-data text-sm"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-cloud-elements-textSecondary">Public Key</label>
                <Textarea
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

      {/* Secrets Tab — inject and wipe environment secrets */}
      {tab === 'secrets' && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Environment Secrets</CardTitle>
              <CardDescription>Inject environment variables as secrets into the sandbox</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-cloud-elements-textSecondary">
                  Secrets (JSON object)
                </label>
                <Textarea
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
                <Button
                  size="sm"
                  onClick={handleInjectSecrets}
                  disabled={secretsBusy}
                >
                  {secretsBusy ? 'Injecting...' : 'Inject Secrets'}
                </Button>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={handleWipeSecrets}
                  disabled={secretsBusy}
                >
                  Wipe All Secrets
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Attestation Tab — TEE attestation verification */}
      {tab === 'attestation' && (
        <div className="space-y-4">
          <TeeAttestationCard
            subjectLabel="sandbox"
            attestation={attestation}
            busy={attestationBusy}
            error={attestationError}
            onFetch={handleFetchAttestation}
          />
        </div>
      )}
      <ConfirmDialog
        open={!!confirmAction}
        onOpenChange={(open) => { if (!open) setConfirmAction(null); }}
        title={confirmAction?.title ?? ''}
        description={confirmAction?.description ?? ''}
        confirmLabel={confirmAction?.confirmLabel ?? 'Confirm'}
        onConfirm={() => confirmAction?.onConfirm()}
        variant="danger"
      />
    </AnimatedPage>
  );
}
