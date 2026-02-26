import { useParams, Link } from 'react-router';
import { lazy, Suspense, useState, useCallback, useMemo } from 'react';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Input } from '~/components/ui/input';
import { Textarea } from '~/components/ui/textarea';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import { sandboxListStore, updateSandboxStatus } from '~/lib/stores/sandboxes';
import { useSandboxActive, useSandboxOperator } from '~/lib/hooks/useSandboxReads';
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getJobById } from '~/lib/blueprints';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import '~/lib/blueprints'; // auto-register
import { useWagmiSidecarAuth } from '~/lib/hooks/useWagmiSidecarAuth';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { createDirectClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { cn } from '~/lib/utils';

const TerminalView = lazy(() =>
  import('@tangle/agent-ui/terminal').then((m) => ({ default: m.TerminalView }))
);

type ActionTab = 'overview' | 'terminal' | 'chat' | 'ssh' | 'secrets';

/** Operator API base URL for sandbox lifecycle operations. */
const OPERATOR_API_URL = import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';
const INSTANCE_OPERATOR_API_URL = import.meta.env.VITE_INSTANCE_OPERATOR_API_URL ?? 'http://localhost:9200';

interface SshKey {
  username: string;
  publicKey: string;
}

export default function SandboxDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const sandboxes = useStore(sandboxListStore);
  const sb = sandboxes.find((s) => s.id === decodedId);

  const { data: isActive } = useSandboxActive(decodedId);
  const { data: operator } = useSandboxOperator(decodedId);
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

  const serviceId = BigInt(sb?.serviceId ?? '1');

  // Resolve correct operator API URL (instance blueprints run on a different port)
  const instanceBpId = import.meta.env.VITE_INSTANCE_BLUEPRINT_ID;
  const teeBpId = import.meta.env.VITE_TEE_INSTANCE_BLUEPRINT_ID;
  const isInstance = sb ? (sb.blueprintId === instanceBpId || sb.blueprintId === teeBpId) : false;
  const operatorUrl = isInstance ? INSTANCE_OPERATOR_API_URL : OPERATOR_API_URL;

  // Sidecar auth for PTY terminal and chat (direct connection)
  const sidecarUrl = sb?.sidecarUrl ?? '';
  const { token: sidecarToken, isAuthenticated: isSidecarAuthed, authenticate: sidecarAuth, isAuthenticating } = useWagmiSidecarAuth(decodedId, sidecarUrl);

  // Operator API auth for lifecycle operations (stop/resume/snapshot/ssh/secrets)
  const { getToken: getOperatorToken } = useOperatorAuth(operatorUrl);

  // Create sandbox client for direct API access (uses authenticated sidecar token)
  const client: SandboxClient | null = useMemo(() => {
    if (!sb?.sidecarUrl || !sidecarToken) return null;
    return createDirectClient(sb.sidecarUrl, sidecarToken);
  }, [sb?.sidecarUrl, sidecarToken]);

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

  /** Call operator API for sandbox lifecycle operations (stop/resume/snapshot/ssh/secrets). */
  const operatorApiCall = useCallback(async (
    action: string,
    body?: Record<string, unknown>,
    opts?: { method?: string },
  ) => {
    const token = await getOperatorToken();
    if (!token) throw new Error('Wallet authentication required');

    // Instance sandboxes use singleton /api/sandbox/* endpoints
    const path = isInstance
      ? `/api/sandbox/${action}`
      : `/api/sandboxes/${encodeURIComponent(decodedId)}/${action}`;
    const url = `${operatorUrl}${path}`;

    const doFetch = (bearerToken: string) =>
      fetch(url, {
        method: opts?.method ?? 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${bearerToken}`,
        },
        body: body ? JSON.stringify(body) : '{}',
      });

    let res = await doFetch(token);

    // Auto-retry once on 401 (expired PASETO token)
    if (res.status === 401) {
      const freshToken = await getOperatorToken(true);
      if (!freshToken) throw new Error('Re-authentication failed');
      res = await doFetch(freshToken);
    }

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`${action} failed (${res.status}): ${text}`);
    }
    return res;
  }, [decodedId, isInstance, operatorUrl, getOperatorToken]);

  const handleStop = useCallback(async () => {
    try {
      await operatorApiCall('stop');
      updateSandboxStatus(decodedId, 'stopped');
    } catch (e) {
      console.error('Stop failed:', e);
      toast.error('Failed to stop sandbox');
    }
  }, [decodedId, operatorApiCall]);

  const handleResume = useCallback(async () => {
    try {
      await operatorApiCall('resume');
      updateSandboxStatus(decodedId, 'running');
    } catch (e) {
      console.error('Resume failed:', e);
      toast.error('Failed to resume sandbox');
    }
  }, [decodedId, operatorApiCall]);

  const handleDelete = useCallback(async () => {
    if (!window.confirm('Are you sure you want to permanently delete this sandbox? This action is irreversible and will submit an on-chain transaction.')) return;
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_DELETE,
      args: encodeCtxJob(JOB_IDS.SANDBOX_DELETE, { sandbox_id: decodedId }),
      label: `Delete: ${decodedId}`,
      value: jobValue(JOB_IDS.SANDBOX_DELETE),
    });
    if (hash) updateSandboxStatus(decodedId, 'gone');
  }, [decodedId, serviceId, submitJob, encodeCtxJob]);

  const handleSnapshot = useCallback(async () => {
    try {
      await operatorApiCall('snapshot', {
        destination: '',
        include_workspace: true,
        include_state: true,
      });
    } catch (e) {
      console.error('Snapshot failed:', e);
      toast.error('Failed to snapshot sandbox');
    }
  }, [operatorApiCall]);

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
      setTimeout(() => setSshSuccess(null), 3000);
    } catch (e) {
      setSshError(e instanceof Error ? e.message : 'Failed to provision SSH key');
    } finally {
      setSshBusy(false);
    }
  }, [sshUsername, sshPublicKey, operatorApiCall]);

  const handleSshRevoke = useCallback(async (key: SshKey) => {
    setSshBusy(true);
    setSshError(null);
    setSshSuccess(null);
    try {
      await operatorApiCall('ssh', { username: key.username, public_key: key.publicKey }, { method: 'DELETE' });
      setSshKeys((prev) => prev.filter((k) => k.publicKey !== key.publicKey));
      setSshSuccess('SSH key revoked');
      setTimeout(() => setSshSuccess(null), 3000);
    } catch (e) {
      setSshError(e instanceof Error ? e.message : 'Failed to revoke SSH key');
    } finally {
      setSshBusy(false);
    }
  }, [operatorApiCall]);

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
      setTimeout(() => setSecretsSuccess(null), 3000);
    } catch (e) {
      setSecretsError(e instanceof Error ? e.message : 'Failed to inject secrets');
    } finally {
      setSecretsBusy(false);
    }
  }, [secretsJson, operatorApiCall]);

  const handleWipeSecrets = useCallback(async () => {
    if (!window.confirm('Are you sure you want to wipe all secrets? This will restart the sandbox without any injected secrets.')) return;
    setSecretsBusy(true);
    setSecretsError(null);
    setSecretsSuccess(null);
    try {
      await operatorApiCall('secrets', undefined, { method: 'DELETE' });
      setSecretsSuccess('Secrets wiped');
      setTimeout(() => setSecretsSuccess(null), 3000);
    } catch (e) {
      setSecretsError(e instanceof Error ? e.message : 'Failed to wipe secrets');
    } finally {
      setSecretsBusy(false);
    }
  }, [operatorApiCall]);

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

  const isRunning = sb.status === 'running' || sb.status === 'creating';
  const isStopped = sb.status === 'stopped' || sb.status === 'warm';
  const isGone = sb.status === 'gone';

  const tabs: { key: ActionTab; label: string; icon: string; disabled?: boolean }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal', disabled: !isRunning },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle', disabled: !isRunning },
    { key: 'ssh', label: 'SSH', icon: 'i-ph:key', disabled: !isRunning },
    { key: 'secrets', label: 'Secrets', icon: 'i-ph:lock-simple', disabled: !isRunning },
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
              'i-ph:hard-drives text-2xl',
              isRunning ? 'text-teal-400' : isStopped ? 'text-amber-400' : 'text-cloud-elements-textTertiary',
            )} />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-xl font-display font-bold text-cloud-elements-textPrimary">{sb.name}</h1>
              <StatusBadge status={sb.status === 'creating' ? 'running' : sb.status} />
            </div>
            <div className="flex items-center gap-3 mt-1">
              <span className="text-xs font-data text-cloud-elements-textTertiary">{sb.image}</span>
              <span className="text-cloud-elements-dividerColor">·</span>
              <span className="text-xs font-data text-cloud-elements-textTertiary">
                {sb.cpuCores} CPU · {sb.memoryMb}MB · {sb.diskGb}GB
              </span>
            </div>
          </div>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          {isRunning && (
            <Button variant="secondary" size="sm" onClick={handleStop}>
              <div className="i-ph:stop text-sm" />
              Stop
            </Button>
          )}
          {isStopped && (
            <Button variant="success" size="sm" onClick={handleResume}>
              <div className="i-ph:play text-sm" />
              Resume
            </Button>
          )}
          {!isGone && (
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

      {/* Tabs */}
      <div className="flex items-center gap-1 mb-6 border-b border-cloud-elements-dividerColor pb-px">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => !t.disabled && setTab(t.key)}
            disabled={t.disabled}
            className={cn(
              'flex items-center gap-1.5 px-3 py-2 text-sm font-display font-medium transition-colors border-b-2 -mb-px',
              tab === t.key
                ? 'text-violet-700 dark:text-violet-400 border-violet-500'
                : t.disabled
                  ? 'text-cloud-elements-textTertiary border-transparent cursor-not-allowed opacity-50'
                  : 'text-cloud-elements-textSecondary border-transparent hover:text-cloud-elements-textPrimary hover:border-cloud-elements-borderColor',
            )}
          >
            <div className={`${t.icon} text-sm`} />
            {t.label}
          </button>
        ))}
      </div>

      {/* Provision Progress (shown when creating) */}
      {sb.status === 'creating' && sb.callId && (
        <ProvisionProgress
          callId={sb.callId}
          className="mb-4"
          onReady={(sandboxId, sidecarUrl) => {
            updateSandboxStatus(decodedId, 'running', { id: sandboxId, sidecarUrl });
          }}
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
              <DetailRow label="Sandbox ID" value={sb.id} mono />
              <DetailRow label="Image" value={sb.image} mono />
              <DetailRow label="CPU" value={`${sb.cpuCores} cores`} />
              <DetailRow label="Memory" value={`${sb.memoryMb} MB`} />
              <DetailRow label="Disk" value={`${sb.diskGb} GB`} />
              <DetailRow label="Created" value={new Date(sb.createdAt).toLocaleString()} />
              <DetailRow label="Blueprint" value={sb.blueprintId} mono />
              <DetailRow label="Service ID" value={sb.serviceId} />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">On-Chain Status</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2.5">
              <DetailRow label="Active" value={isActive !== undefined ? (isActive ? 'Yes' : 'No') : 'Loading...'} />
              <DetailRow
                label="Operator"
                value={operator && operator !== '0x0000000000000000000000000000000000000000' ? operator : 'Unassigned'}
                mono
              />
              {sb.txHash && <DetailRow label="TX Hash" value={sb.txHash} mono />}
              {sb.sidecarUrl ? (
                <DetailRow label="Sidecar" value={sb.sidecarUrl} mono />
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
              <div className="h-[500px]">
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
            <div className="h-[600px]">
              {!isSidecarAuthed ? (
                <div className="flex flex-col items-center justify-center h-full gap-3">
                  <div className="i-ph:chat-circle text-3xl text-cloud-elements-textTertiary" />
                  <p className="text-sm text-cloud-elements-textSecondary">
                    Authenticate to start chatting
                  </p>
                  <p className="text-xs text-cloud-elements-textTertiary">
                    Sign a message with your wallet to verify ownership
                  </p>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => sidecarAuth()}
                    disabled={isAuthenticating || !sidecarUrl}
                  >
                    {isAuthenticating ? 'Signing...' : !sidecarUrl ? 'Waiting for sidecar...' : 'Connect'}
                  </Button>
                </div>
              ) : (
                <SessionSidebar
                  sandboxId={decodedId}
                  client={client}
                  systemPrompt={systemPrompt}
                  onSystemPromptChange={setSystemPrompt}
                />
              )}
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
    </AnimatedPage>
  );
}

function DetailRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between text-sm gap-2">
      <span className="text-cloud-elements-textSecondary shrink-0">{label}</span>
      <span className={cn('text-cloud-elements-textPrimary truncate text-right', mono && 'font-data text-xs')}>
        {value}
      </span>
    </div>
  );
}
