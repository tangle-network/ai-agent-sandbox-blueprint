import { useParams, Link, useNavigate } from 'react-router';
import { useState, useCallback, useMemo, useRef, useEffect } from 'react';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { ResourceTabs } from '~/components/shared/ResourceTabs';
import {
  sandboxListStore,
  findSandboxByKey,
  getSandboxRouteKey,
  updateSandboxStatus,
} from '~/lib/stores/sandboxes';
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
import { ConfirmDialog } from '~/components/shared/ConfirmDialog';
import { SnapshotDialog } from '~/components/shared/SnapshotDialog';
import { ResourceWorkspaceNav } from '~/components/console/ResourceWorkspaceNav';
import { ResourceWorkspaceRail } from '~/components/console/ResourceWorkspacePanels';

import { useAccount } from 'wagmi';
import { normalizeAgentIdentifier } from '~/lib/agents';

import {
  INSTANCE_ONCHAIN_BLUEPRINT_ID,
  INSTANCE_ONCHAIN_SERVICE_ID,
  INSTANCE_OPERATOR_API_URL,
  OPERATOR_API_URL,
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
  TEE_INSTANCE_ONCHAIN_SERVICE_ID,
} from '~/lib/config';

import { OverviewTab } from '~/components/sandbox-detail/overview';
import { AutomationTab } from '~/components/sandbox-detail/automation';
import { StorageTab } from '~/components/sandbox-detail/storage';
import { TerminalTab } from '~/components/sandbox-detail/terminal';
import { ChatTab } from '~/components/sandbox-detail/chat';
import { SshTab } from '~/components/sandbox-detail/ssh';
import { SecretsTab } from '~/components/sandbox-detail/secrets';
import { AttestationTab } from '~/components/sandbox-detail/attestation';
import { SandboxAlerts, SandboxHeader } from '~/components/sandbox-detail/header';
import { buildSandboxSummary } from '~/components/sandbox-detail/summary';
import {
  type ActionTab,
  type AgentDescriptor,
  type SshKey,
  getCurrentPathname,
  getInitialTabFromPath,
  parseApiError,
} from '~/components/sandbox-detail/helpers';

export default function SandboxDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const currentPathname = getCurrentPathname();
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

  const [tab, setTab] = useState<ActionTab>(() => getInitialTabFromPath(currentPathname));
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
  const isInstance = sb
    ? (
        sb.blueprintId === INSTANCE_ONCHAIN_BLUEPRINT_ID
        || sb.blueprintId === TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID
        || sb.serviceId === INSTANCE_ONCHAIN_SERVICE_ID
        || (!!TEE_INSTANCE_ONCHAIN_SERVICE_ID && sb.serviceId === TEE_INSTANCE_ONCHAIN_SERVICE_ID)
      )
    : false;
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
    verification: attestationVerification,
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

  const { tabs, workspaceNavItems, workspaceMetrics, contextRows, storageRows, workflowCreateHref } =
    buildSandboxSummary({
      sb,
      isRunning,
      isCreating,
      isStopped,
      isGone,
      hasProvisionedSandbox,
      hasAgent,
      configuredAgentIdentifier,
      operatorUrl,
      ports,
      tab,
      currentPathname,
      routeKey,
      attestationVerification,
    });

  return (
    <AnimatedPage className="mx-auto max-w-5xl px-4 sm:px-6 py-8">
      <SandboxHeader
        sb={sb}
        hasProvisionedSandbox={hasProvisionedSandbox}
        isRunning={isRunning}
        isCreating={isCreating}
        isStopped={isStopped}
        isGone={isGone}
        hasAgent={hasAgent}
        handleStop={handleStop}
        handleResume={handleResume}
        setSnapshotOpen={setSnapshotOpen}
        handleDelete={handleDelete}
        workspaceMetrics={workspaceMetrics}
      />

      <div className="mb-4 grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0 space-y-4">
          <ResourceWorkspaceNav items={workspaceNavItems} activePath={currentPathname} />
          <ResourceTabs tabs={tabs} value={tab} onValueChange={setTab} className="mb-0" />

      <SandboxAlerts
        sb={sb}
        routeKey={routeKey}
        agentConfigured={agentConfigured}
        hasAgentValidationResult={hasAgentValidationResult}
        agentIdentifierValid={agentIdentifierValid}
        configuredAgentIdentifier={configuredAgentIdentifier}
        agentAvailableList={agentAvailableList}
      />

      {/* Tab Content */}
      {tab === 'overview' && (
        <OverviewTab sb={sb} ports={ports} operatorUrl={operatorUrl} />
      )}

      {tab === 'automation' && (
        <AutomationTab sb={sb} workflowCreateHref={workflowCreateHref} hasAgent={hasAgent} />
      )}

      {tab === 'storage' && (
        <StorageTab
          rows={storageRows}
          onSnapshot={() => setSnapshotOpen(true)}
          snapshotEnabled={hasProvisionedSandbox && !isGone}
        />
      )}

      {/* Terminal Tab — operator-backed terminal */}
      {tab === 'terminal' && (
        <TerminalTab
          isOperatorAuthed={isOperatorAuthed}
          operatorToken={operatorToken}
          operatorAuthError={operatorAuthError}
          onAuthenticate={handleOperatorAuthenticate}
          isOperatorAuthenticating={isOperatorAuthenticating}
          hasWallet={hasWallet}
          hasProvisionedSandbox={hasProvisionedSandbox}
          sshUserDetecting={sshUserDetecting}
          operatorUrl={operatorUrl}
          operatorResourcePath={operatorResourcePath}
          terminalPath={terminalPath}
          terminalUsername={terminalUsername}
        />
      )}

      {/* Chat Tab — multi-session agent chat */}
      {tab === 'chat' && (
        <ChatTab
          isOperatorAuthed={isOperatorAuthed}
          agentConfigured={agentConfigured}
          agentDiscoveryLoading={agentDiscoveryLoading}
          hasAgentValidationResult={hasAgentValidationResult}
          agentIdentifierValid={agentIdentifierValid}
          configuredAgentIdentifier={configuredAgentIdentifier}
          agentAvailableList={agentAvailableList}
          sb={sb}
          operatorAuthError={operatorAuthError}
          handleOperatorAuthenticate={handleOperatorAuthenticate}
          isOperatorAuthenticating={isOperatorAuthenticating}
          hasWallet={hasWallet}
          hasProvisionedSandbox={hasProvisionedSandbox}
          setTab={setTab}
          agentDiscoveryError={agentDiscoveryError}
          canonicalSandboxId={canonicalSandboxId}
          client={client}
          systemPrompt={systemPrompt}
          setSystemPrompt={setSystemPrompt}
        />
      )}

      {/* SSH Tab — provision and revoke SSH keys */}
      {tab === 'ssh' && (
        <SshTab
          sshConnectionCommand={sshConnectionCommand}
          sshUsername={sshUsername}
          setSshUsername={setSshUsername}
          sshUsernameDirtyRef={sshUsernameDirtyRef}
          sshUserDetecting={sshUserDetecting}
          sshUserHint={sshUserHint}
          sshPublicKey={sshPublicKey}
          setSshPublicKey={setSshPublicKey}
          sshError={sshError}
          sshSuccess={sshSuccess}
          handleSshProvision={handleSshProvision}
          sshBusy={sshBusy}
          sshKeys={sshKeys}
          handleSshRevoke={handleSshRevoke}
        />
      )}

      {/* Secrets Tab — inject and wipe environment secrets */}
      {tab === 'secrets' && (
        <SecretsTab
          secretsVisible={secretsVisible}
          setSecretsVisible={setSecretsVisible}
          secretsLoading={secretsLoading}
          secretsJson={secretsJson}
          setSecretsJson={setSecretsJson}
          secretsError={secretsError}
          secretsSuccess={secretsSuccess}
          handleInjectSecrets={handleInjectSecrets}
          secretsBusy={secretsBusy}
          handleWipeSecrets={handleWipeSecrets}
        />
      )}

      {/* Attestation Tab — TEE attestation verification */}
      {tab === 'attestation' && (
        <AttestationTab
          attestation={attestation}
          attestationVerification={attestationVerification}
          attestationBusy={attestationBusy}
          attestationError={attestationError}
          handleFetchAttestation={handleFetchAttestation}
        />
      )}
        </div>
        <ResourceWorkspaceRail rows={contextRows} />
      </div>
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
