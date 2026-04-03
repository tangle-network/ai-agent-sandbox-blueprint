import { useParams, Link, useNavigate } from 'react-router';
import { useState, useCallback, useMemo, useRef, useEffect } from 'react';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Textarea } from '@tangle-network/blueprint-ui/components';
import { getBlueprint } from '@tangle-network/blueprint-ui';
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
import { ProvisionProgress } from '~/components/shared/ProvisionProgress';
import { useSubmitJob } from '@tangle-network/blueprint-ui';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { useOperatorApiCall } from '~/lib/hooks/useOperatorApiCall';
import { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { useTeeAttestation } from '~/lib/hooks/useTeeAttestation';
import { useSandboxHydration } from '~/lib/hooks/useSandboxHydration';
import { createProxiedClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { cn } from '@tangle-network/blueprint-ui';
import { truncateAddress } from '~/lib/utils/truncate-address';
import { ConfirmDialog } from '~/components/shared/ConfirmDialog';
import { SnapshotDialog } from '~/components/shared/SnapshotDialog';
import { OperatorTerminalView } from '~/components/shared/OperatorTerminalView';

import { useAccount } from 'wagmi';
import { normalizeAgentIdentifier } from '~/lib/agents';

type ActionTab = 'overview' | 'terminal' | 'chat' | 'ssh' | 'secrets' | 'attestation';

import { OPERATOR_API_URL, INSTANCE_OPERATOR_API_URL } from '~/lib/config';

interface SshKey {
  username: string;
  publicKey: string;
}

interface AgentDescriptor {
  identifier: string;
  displayName?: string;
  description?: string;
}

/** Extract human-readable error from operator API Error messages. */
function parseApiError(err: Error): string {
  const idx = err.message.indexOf('): ');
  if (idx === -1) return err.message;
  const body = err.message.slice(idx + 3);
  try {
    const parsed = JSON.parse(body);
    if (typeof parsed.error === 'string') return parsed.error;
  } catch { /* not JSON */ }
  return err.message;
}

function formatBlueprintLabel(blueprintId: string): string {
  const id = blueprintId.trim() || 'ai-agent-sandbox-blueprint';
  return getBlueprint(id)?.name ?? id;
}

function formatServiceId(serviceId: string): string {
  const trimmed = serviceId.trim();
  if (!trimmed) return 'Not linked';
  if (trimmed.startsWith('#')) return trimmed;
  return /^\d+$/.test(trimmed) ? `#${trimmed}` : trimmed;
}

function formatDuration(seconds: number): string {
  if (seconds <= 0) return 'Unlimited';
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0 && minutes > 0) return `${hours}h ${minutes}m`;
  if (hours > 0) return `${hours} hour${hours !== 1 ? 's' : ''}`;
  if (minutes > 0) return `${minutes} minute${minutes !== 1 ? 's' : ''}`;
  return `${seconds}s`;
}

export default function SandboxDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const decodedKey = id ? decodeURIComponent(id) : '';
  const sandboxes = useStore(sandboxListStore);
  const sb = findSandboxByKey(sandboxes, decodedKey);
  const canonicalSandboxId = sb?.sandboxId;
  const routeKey = sb ? getSandboxRouteKey(sb) : decodedKey;
  const isRunning = sb?.status === 'running';
  const sshHost = useMemo(() => {
    if (!sb?.sidecarUrl) return '';
    try {
      return new URL(sb.sidecarUrl).hostname;
    } catch {
      return '';
    }
  }, [sb?.sidecarUrl]);

  const { submitJob } = useSubmitJob();
  const { address } = useAccount();

  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');
  // SSH state
  const [sshPublicKey, setSshPublicKey] = useState('');
  const [sshUsername, setSshUsername] = useState('');
  const [sshKeys, setSshKeys] = useState<SshKey[]>([]);
  const [sshBusy, setSshBusy] = useState(false);
  const [sshError, setSshError] = useState<string | null>(null);
  const [sshSuccess, setSshSuccess] = useState<string | null>(null);
  const [detectedSshUsername, setDetectedSshUsername] = useState('');
  const [sshUserDetecting, setSshUserDetecting] = useState(false);
  const [sshUserHint, setSshUserHint] = useState<string | null>(null);
  const sshUsernameDirtyRef = useRef(false);
  const sshUserDetectionKeyRef = useRef<string | null>(null);

  // Secrets state
  const [secretsJson, setSecretsJson] = useState('{\n  \n}');
  const [secretsVisible, setSecretsVisible] = useState(false);
  const [secretsBusy, setSecretsBusy] = useState(false);
  const [secretsError, setSecretsError] = useState<string | null>(null);
  const [secretsSuccess, setSecretsSuccess] = useState<string | null>(null);
  const [secretsLoading, setSecretsLoading] = useState(false);
  const secretsFetchedRef = useRef(false);

  // Confirm dialog state
  const [confirmAction, setConfirmAction] = useState<{ title: string; description: string; confirmLabel: string; onConfirm: () => void } | null>(null);

  // Snapshot dialog state
  const [snapshotOpen, setSnapshotOpen] = useState(false);
  const [availableAgents, setAvailableAgents] = useState<AgentDescriptor[] | null>(null);
  const [agentDiscoveryLoading, setAgentDiscoveryLoading] = useState(false);
  const [agentDiscoveryError, setAgentDiscoveryError] = useState<string | null>(null);

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
  const sshConnectionCommand = useMemo(() => {
    if (!sb?.sshPort || !sshHost) return '';
    const user = sshUsername.trim() || 'sidecar';
    return `ssh ${user}@${sshHost} -p ${sb.sshPort}`;
  }, [sb?.sshPort, sshHost, sshUsername]);
  const terminalUsername = detectedSshUsername.trim() || 'agent';
  const terminalPath = `/home/${terminalUsername}`;

  // Resolve correct operator API URL (instance blueprints run on a different port)
  const instanceBpId = import.meta.env.VITE_INSTANCE_BLUEPRINT_ID;
  const teeBpId = import.meta.env.VITE_TEE_INSTANCE_BLUEPRINT_ID;
  const isInstance = sb ? (sb.blueprintId === instanceBpId || sb.blueprintId === teeBpId) : false;
  const operatorUrl = isInstance ? (INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL) : OPERATOR_API_URL;

  // Operator API auth for lifecycle operations and browser access to live features.
  const {
    getToken: getOperatorToken,
    getCachedToken: getCachedOperatorToken,
    isAuthenticated: isOperatorAuthed,
    isAuthenticating: isOperatorAuthenticating,
    error: operatorAuthError,
  } = useOperatorAuth(operatorUrl);
  const buildPath = useCallback(
    (action: string) =>
      isInstance
        ? `/api/sandbox/${action}`
        : `/api/sandboxes/${encodeURIComponent(canonicalSandboxId ?? '__draft__')}/${action}`,
    [canonicalSandboxId, isInstance],
  );
  const operatorApiCall = useOperatorApiCall(operatorUrl, getOperatorToken, buildPath);
  const ports = useExposedPorts(canonicalSandboxId ? sb?.status : undefined, operatorApiCall);
  const { refresh: refreshSandboxState } = useSandboxHydration();
  const operatorResourcePath = useMemo(
    () =>
      isInstance
        ? '/api/sandbox'
        : `/api/sandboxes/${encodeURIComponent(canonicalSandboxId ?? '__draft__')}`,
    [canonicalSandboxId, isInstance],
  );
  const operatorToken = getCachedOperatorToken();
  const hasWallet = !!address;
  const sshDetectionKey = isInstance ? 'instance' : canonicalSandboxId ?? null;
  const configuredAgentIdentifier = normalizeAgentIdentifier(sb?.agentIdentifier);
  const agentConfigured = configuredAgentIdentifier.length > 0;

  // Chat client: always proxied through the operator API.
  const client: SandboxClient | null = useMemo(() => {
    if (!canonicalSandboxId) return null;
    return createProxiedClient(canonicalSandboxId, getOperatorToken, operatorUrl);
  }, [canonicalSandboxId, getOperatorToken, operatorUrl]);

  const handleOperatorAuthenticate = useCallback(() => {
    void getOperatorToken();
  }, [getOperatorToken]);

  useEffect(() => {
    if (!sb?.sandboxId) return;
    if (decodedKey === sb.sandboxId) return;
    navigate(`/sandboxes/${encodeURIComponent(sb.sandboxId)}`, { replace: true });
  }, [sb?.sandboxId, decodedKey, navigate]);

  useEffect(() => {
    sshUsernameDirtyRef.current = false;
    sshUserDetectionKeyRef.current = null;
    setDetectedSshUsername('');
    setSshUsername('');
    setSshUserHint(null);
    setSshUserDetecting(false);
  }, [sshDetectionKey]);

  useEffect(() => {
    if (!agentConfigured || !isRunning || !canonicalSandboxId) {
      setAvailableAgents(null);
      setAgentDiscoveryLoading(false);
      setAgentDiscoveryError(null);
      return;
    }
    if (!isOperatorAuthed && !operatorToken) {
      setAvailableAgents(null);
      setAgentDiscoveryLoading(false);
      setAgentDiscoveryError(null);
      return;
    }

    let cancelled = false;
    setAgentDiscoveryLoading(true);
    setAgentDiscoveryError(null);

    void operatorApiCall('agents', undefined, { method: 'GET' })
      .then(async (response) => {
        const body = await response.json() as { agents?: AgentDescriptor[] };
        if (cancelled) return;
        setAvailableAgents(Array.isArray(body.agents) ? body.agents : []);
      })
      .catch((e) => {
        if (cancelled) return;
        setAvailableAgents(null);
        setAgentDiscoveryError(
          e instanceof Error ? parseApiError(e) : 'Could not load available agents.',
        );
      })
      .finally(() => {
        if (!cancelled) {
          setAgentDiscoveryLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [agentConfigured, canonicalSandboxId, isOperatorAuthed, isRunning, operatorApiCall, operatorToken]);

  useEffect(() => {
    if ((tab !== 'ssh' && tab !== 'terminal') || !sshDetectionKey) return;
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
          setDetectedSshUsername(detectedUsername);
          if (!sshUsernameDirtyRef.current) {
            setSshUsername(detectedUsername);
          }
          setSshUserHint(`Detected sandbox user: ${detectedUsername}`);
        } else {
          setDetectedSshUsername('');
          setSshUserHint('Could not detect the sandbox user. You can enter one manually.');
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setDetectedSshUsername('');
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
      const refreshed = await refreshSandboxState({ interactive: true });
      if (!refreshed) {
        toast.error('Sandbox resumed, but the latest state could not be refreshed');
      }
    } catch (e) {
      console.error('Resume failed:', e);
      toast.error('Failed to resume sandbox');
    }
  }, [canonicalSandboxId, operatorApiCall, refreshSandboxState]);

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

  const handleSnapshot = useCallback(
    async (params: { destination: string; include_workspace: boolean; include_state: boolean }) => {
      if (!canonicalSandboxId) return;
      await operatorApiCall('snapshot', params);
      toast.success('Snapshot created');
    },
    [canonicalSandboxId, operatorApiCall],
  );

  // SSH handlers
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
      await refreshSandboxState({ interactive: true });
      setSecretsSuccess('Secrets injected');
      scheduleDismiss(() => setSecretsSuccess(null), 3000);
    } catch (e) {
      setSecretsError(e instanceof Error ? e.message : 'Failed to inject secrets');
    } finally {
      setSecretsBusy(false);
    }
  }, [secretsJson, operatorApiCall, refreshSandboxState, scheduleDismiss]);

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
          await refreshSandboxState({ interactive: true });
          setSecretsJson('{\n  \n}');
          secretsFetchedRef.current = false;
          setSecretsSuccess('Secrets wiped');
          scheduleDismiss(() => setSecretsSuccess(null), 3000);
        } catch (e) {
          setSecretsError(e instanceof Error ? e.message : 'Failed to wipe secrets');
        } finally {
          setSecretsBusy(false);
        }
      },
    });
  }, [operatorApiCall, refreshSandboxState, scheduleDismiss]);

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
  const isStopped = sb.status === 'stopped' || sb.status === 'warm';
  const isGone = sb.status === 'gone';

  const hasAgent = agentConfigured;
  const agentIdentifiers = availableAgents?.map((agent) => normalizeAgentIdentifier(agent.identifier)) ?? [];
  const agentIdentifierValid = !agentConfigured
    || (availableAgents != null && agentIdentifiers.includes(configuredAgentIdentifier));
  const hasAgentValidationResult = availableAgents != null;
  const agentAvailableList = agentIdentifiers.length > 0 ? agentIdentifiers.join(', ') : 'none reported';

  const tabs: { key: ActionTab; label: string; icon: string; disabled?: boolean; hidden?: boolean }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle', disabled: !hasProvisionedSandbox || !isRunning, hidden: !hasAgent },
    { key: 'ssh', label: 'SSH', icon: 'i-ph:key', disabled: !hasProvisionedSandbox || !isRunning, hidden: !sb.sshPort },
    { key: 'secrets', label: 'Secrets', icon: 'i-ph:lock-simple', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'attestation', label: 'Attestation', icon: 'i-ph:shield-check', hidden: !hasProvisionedSandbox || !sb.teeEnabled },
  ];

  return (
    <AnimatedPage className="mx-auto max-w-5xl px-4 sm:px-6 py-8">
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
              <Button variant="secondary" size="sm" onClick={() => setSnapshotOpen(true)}>
                <div className="i-ph:camera text-sm" />
                Snapshot
              </Button>
              {isRunning && sb.sandboxId && (
                <Link to={`/workflows/create?target=${encodeURIComponent(`sandbox:${sb.sandboxId}`)}`}>
                  <Button variant="secondary" size="sm" title={!hasAgent ? 'No agent configured — workflow executions will fail' : undefined}>
                    <div className="i-ph:flow-arrow text-sm" />
                    Create Workflow
                  </Button>
                </Link>
              )}
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

      {agentConfigured && hasAgentValidationResult && !agentIdentifierValid && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Configured agent not available in this image
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            This sandbox is configured to use <span className="font-data">{configuredAgentIdentifier}</span>, but the running image only reports {agentAvailableList}.
          </p>
        </div>
      )}

      {/* Provision Progress (shown when creating) */}
      {sb.status === 'creating' && sb.callId != null && (
        <ProvisionProgress
          callId={sb.callId}
          className="mb-4"
          onReady={(sandboxId, sidecarUrl) => {
            updateSandboxStatus(routeKey, 'running', { sandboxId, sidecarUrl, errorMessage: undefined });
          }}
          onFailed={(message) => updateSandboxStatus(routeKey, 'error', { errorMessage: message })}
        />
      )}

      {sb.status === 'error' && sb.errorMessage && (
        <div className="mb-4 rounded-xl border border-crimson-500/20 bg-crimson-500/5 p-4">
          <p className="text-sm font-display font-medium text-crimson-300">Provisioning failed</p>
          <p className="mt-1 text-xs text-crimson-200/90">{sb.errorMessage}</p>
        </div>
      )}

      {sb.circuitBreakerActive && (
        <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/5 p-4">
          <p className="text-sm font-display font-medium text-amber-300">
            Sidecar unreachable — circuit breaker active
          </p>
          <p className="mt-1 text-xs text-amber-200/90">
            {sb.circuitBreakerProbing
              ? 'Recovery probe in progress\u2026'
              : `Cooldown active — retrying in ~${sb.circuitBreakerRemainingSecs ?? '?'}s`}
          </p>
        </div>
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
                value={sb.sandboxId && sb.sandboxId.length > 24 ? `${sb.sandboxId.slice(0, 20)}...${sb.sandboxId.slice(-4)}` : (sb.sandboxId || 'Pending operator provision')}
                mono={!!sb.sandboxId}
                copyable={!!sb.sandboxId}
                copyValue={sb.sandboxId ?? undefined}
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
              <LabeledValueRow label="Blueprint" value={formatBlueprintLabel(sb.blueprintId)} alignRight />
              <LabeledValueRow label="Service ID" value={formatServiceId(sb.serviceId)} alignRight />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Runtime Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2.5">
              <LabeledValueRow
                label="Operator"
                value={sb.operator ? truncateAddress(sb.operator) : 'Unknown'}
                mono
                copyable={!!sb.operator}
                copyValue={sb.operator}
                alignRight
              />
              {sb.txHash && <LabeledValueRow label="TX Hash" value={truncateAddress(sb.txHash)} mono copyable copyValue={sb.txHash} alignRight />}
            </CardContent>
          </Card>

          {/* Lifecycle Limits */}
          {(sb.idleTimeoutSeconds != null || sb.maxLifetimeSeconds != null) && (
            <Card>
              <CardHeader>
                <CardTitle className="text-sm">Lifecycle Limits</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2.5">
                {sb.idleTimeoutSeconds != null && (
                  <LabeledValueRow label="Idle Timeout" value={formatDuration(sb.idleTimeoutSeconds)} alignRight />
                )}
                {sb.maxLifetimeSeconds != null && (
                  <LabeledValueRow label="Max Lifetime" value={formatDuration(sb.maxLifetimeSeconds)} alignRight />
                )}
                {sb.lastActivityAt != null && (
                  <LabeledValueRow label="Last Activity" value={new Date(sb.lastActivityAt).toLocaleString()} alignRight />
                )}
                {sb.maxLifetimeSeconds != null && sb.maxLifetimeSeconds > 0 && (
                  <LabeledValueRow
                    label="Expires At"
                    value={(() => {
                      const expiresAt = sb.createdAt + sb.maxLifetimeSeconds * 1000;
                      return expiresAt < Date.now() ? 'Expired' : new Date(expiresAt).toLocaleString();
                    })()}
                    alignRight
                  />
                )}
              </CardContent>
            </Card>
          )}

          {/* Exposed Ports */}
          {ports && ports.length > 0 && (
            <ExposedPortsCard
              ports={ports}
              proxyBaseUrl={`${operatorUrl}/api/sandboxes/${sb.sandboxId}/port/`}
              className="md:col-span-2"
            />
          )}
        </div>
      )}

      {/* Terminal Tab — operator-backed terminal */}
      {tab === 'terminal' && (
        <Card className="overflow-hidden">
          {!isOperatorAuthed || !operatorToken ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:terminal-window text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                Authenticate with the operator to access the sandbox terminal
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-4">
                The browser talks only to the operator API, which verifies sandbox ownership before relaying commands.
              </p>
              {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
              <Button
                variant="secondary"
                size="sm"
                onClick={handleOperatorAuthenticate}
                disabled={isOperatorAuthenticating || !hasWallet || !hasProvisionedSandbox}
              >
                {isOperatorAuthenticating
                  ? 'Signing...'
                  : !hasWallet
                    ? 'Connect Wallet First'
                    : !hasProvisionedSandbox
                      ? 'Waiting for Sandbox...'
                      : 'Connect Terminal'}
              </Button>
            </CardContent>
          ) : sshUserDetecting ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:terminal-window text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                Preparing the sandbox terminal
              </p>
              <p className="text-xs text-cloud-elements-textTertiary">
                Resolving the sandbox user so Terminal starts in the same home directory as SSH.
              </p>
            </CardContent>
          ) : (
            <CardContent className="p-0">
              <div className="h-[min(500px,60vh)]">
                <OperatorTerminalView
                  apiUrl={operatorUrl}
                  resourcePath={operatorResourcePath}
                  token={operatorToken}
                  title="Sandbox Shell"
                  subtitle="Secure shell via operator relay"
                  initialCwd={terminalPath}
                  displayUsername={terminalUsername}
                  displayPath={terminalPath}
                />
              </div>
            </CardContent>
          )}
        </Card>
      )}

      {/* Chat Tab — multi-session agent chat */}
      {tab === 'chat' && (
        <Card className="overflow-hidden">
          {!isOperatorAuthed ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:chat-circle text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                Authenticate with the operator to chat with the sandbox agent
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-4">
                Chat requests are proxied through the operator API and no longer connect directly to sandbox containers.
              </p>
              {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
              <Button
                variant="secondary"
                size="sm"
                onClick={handleOperatorAuthenticate}
                disabled={isOperatorAuthenticating || !hasWallet || !hasProvisionedSandbox}
              >
                {isOperatorAuthenticating
                  ? 'Signing...'
                  : !hasWallet
                    ? 'Connect Wallet First'
                    : !hasProvisionedSandbox
                      ? 'Waiting for Sandbox...'
                      : 'Authenticate to Chat'}
              </Button>
            </CardContent>
          ) : agentConfigured && agentDiscoveryLoading && !hasAgentValidationResult ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:spinner-gap text-3xl text-cloud-elements-textTertiary mb-3 mx-auto animate-spin" />
              <p className="text-sm text-cloud-elements-textSecondary">
                Checking which agents this image exposes...
              </p>
            </CardContent>
          ) : agentConfigured && hasAgentValidationResult && !agentIdentifierValid ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:warning-circle text-3xl text-amber-400 mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                The configured agent is not available in this sandbox image
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-2">
                Configured agent: <span className="font-data">{configuredAgentIdentifier}</span>
              </p>
              <p className="text-xs text-cloud-elements-textTertiary">
                Available agents: <span className="font-data">{agentAvailableList}</span>
              </p>
            </CardContent>
          ) : sb.credentialsAvailable === false ? (
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
              {agentDiscoveryError && (
                <div className="border-b border-amber-500/20 bg-amber-500/5 px-3 py-2">
                  <p className="text-xs text-amber-300">{agentDiscoveryError}</p>
                </div>
              )}
              <div className="h-[min(600px,65vh)]">
                <SessionSidebar
                  sandboxId={canonicalSandboxId ?? sb.localId}
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
                  Docker sandboxes are usually reachable from this machine only unless the runtime exposes them more broadly.
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
                <div className="flex items-center justify-between">
                  <label className="text-xs font-medium text-cloud-elements-textSecondary">
                    Secrets (JSON object)
                  </label>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 w-7 p-0"
                    onClick={() => setSecretsVisible((v) => !v)}
                    title={secretsVisible ? 'Hide secrets' : 'Show secrets'}
                  >
                    <div className={cn('text-sm', secretsVisible ? 'i-ph:eye' : 'i-ph:eye-slash')} />
                  </Button>
                </div>
                {secretsLoading && (
                  <p className="text-xs text-cloud-elements-textTertiary">Loading existing secrets...</p>
                )}
                <Textarea
                  value={secretsJson}
                  onChange={(e) => setSecretsJson(e.target.value)}
                  placeholder='{"API_KEY": "sk-...", "DB_URL": "postgres://..."}'
                  className="font-data text-xs min-h-[120px] resize-y"
                  style={{ filter: secretsVisible ? 'none' : 'blur(4px)' }}
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
                <Button
                  size="sm"
                  onClick={handleInjectSecrets}
                  disabled={secretsBusy || secretsLoading}
                >
                  {secretsBusy ? 'Injecting...' : 'Inject Secrets'}
                </Button>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={handleWipeSecrets}
                  disabled={secretsBusy || secretsLoading}
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
      <SnapshotDialog
        open={snapshotOpen}
        onOpenChange={setSnapshotOpen}
        onConfirm={handleSnapshot}
      />
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
