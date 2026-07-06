import { useParams, Link } from 'react-router';
import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { ResourceTabs } from '~/components/shared/ResourceTabs';
import { instanceListStore, getInstance, updateInstanceStatus } from '~/lib/stores/instances';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { useOperatorApiCall } from '~/lib/hooks/useOperatorApiCall';
import { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { useTeeAttestation } from '~/lib/hooks/useTeeAttestation';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { useInstanceHydration } from '~/lib/hooks/useInstanceHydration';
import { createProxiedInstanceClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { ConfirmDialog } from '~/components/shared/ConfirmDialog';
import { SnapshotDialog } from '~/components/shared/SnapshotDialog';
import { ResourceWorkspaceNav } from '~/components/console/ResourceWorkspaceNav';
import { ResourceWorkspaceRail } from '~/components/console/ResourceWorkspacePanels';
import { useAccount } from 'wagmi';
import { normalizeAgentIdentifier } from '~/lib/agents';

import { InstanceHeader, InstanceAlerts } from '~/components/instance-detail/header';
import { OverviewTab } from '~/components/instance-detail/overview';
import { AutomationTab } from '~/components/instance-detail/automation';
import { StorageTab } from '~/components/instance-detail/storage';
import { TerminalTab } from '~/components/instance-detail/terminal';
import { ChatTab } from '~/components/instance-detail/chat';
import { SshTab } from '~/components/instance-detail/ssh';
import { SecretsTab } from '~/components/instance-detail/secrets';
import { AttestationTab } from '~/components/instance-detail/attestation';
import { buildInstanceSummary } from '~/components/instance-detail/summary';
import {
  type ActionTab,
  type AgentDescriptor,
  type SshKey,
  getCurrentPathname,
  getInitialTabFromPath,
  parseApiError,
} from '~/components/instance-detail/helpers';

export default function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const currentPathname = getCurrentPathname();
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

  const [tab, setTab] = useState<ActionTab>(() => getInitialTabFromPath(currentPathname));
  const [systemPrompt, setSystemPrompt] = useState('');
  const [secretsJson, setSecretsJson] = useState('{\n  \n}');
  const [secretsVisible, setSecretsVisible] = useState(false);
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
  const [detectedSshUsername, setDetectedSshUsername] = useState('');
  const [sshUserDetecting, setSshUserDetecting] = useState(false);
  const [sshUserHint, setSshUserHint] = useState<string | null>(null);
  const sshUsernameDirtyRef = useRef(false);
  const sshUserDetectionKeyRef = useRef<string | null>(null);

  // Agent discovery state
  const [availableAgents, setAvailableAgents] = useState<AgentDescriptor[] | null>(null);
  const [agentDiscoveryLoading, setAgentDiscoveryLoading] = useState(false);
  const [agentDiscoveryError, setAgentDiscoveryError] = useState<string | null>(null);

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

  const configuredAgentIdentifier = normalizeAgentIdentifier(inst?.agentIdentifier);
  const agentConfigured = configuredAgentIdentifier.length > 0;

  const handleSnapshot = useCallback(
    async (params: { destination: string; include_workspace: boolean; include_state: boolean }) => {
      await operatorApiCall('snapshot', params);
    },
    [operatorApiCall],
  );

  // Discover available agents when the instance is running and operator is authed
  useEffect(() => {
    if (!agentConfigured || !isRunning) {
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
  }, [agentConfigured, isOperatorAuthed, isRunning, operatorApiCall, operatorToken]);

  const sshDetectionKey = inst?.sandboxId ?? decodedId;
  const sshConnectionCommand = useMemo(() => {
    if (!inst?.sshPort || !sshHost) return '';
    const user = sshUsername.trim() || 'sidecar';
    return `ssh ${user}@${sshHost} -p ${inst.sshPort}`;
  }, [inst?.sshPort, sshHost, sshUsername]);
  const terminalUsername = detectedSshUsername.trim() || 'agent';
  const terminalPath = `/home/${terminalUsername}`;
  const handleOperatorAuthenticate = useCallback(() => {
    void getOperatorToken();
  }, [getOperatorToken]);

  // Reset SSH username state when switching between instances
  useEffect(() => {
    sshUsernameDirtyRef.current = false;
    sshUserDetectionKeyRef.current = null;
    setDetectedSshUsername('');
    setSshUsername('');
    setSshUserHint(null);
    setSshUserDetecting(false);
  }, [sshDetectionKey]);

  // Auto-detect SSH username when SSH or Terminal opens
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

  const ports = useExposedPorts(inst?.status, operatorApiCall);
  const { refresh: refreshInstances } = useInstanceHydration();

  const {
    attestation,
    verification: attestationVerification,
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

  const hasAgent = agentConfigured;
  const agentIdentifiers = availableAgents?.map((a) => normalizeAgentIdentifier(a.identifier)) ?? [];
  const agentIdentifierValid = !agentConfigured
    || (availableAgents != null && agentIdentifiers.includes(configuredAgentIdentifier));
  const hasAgentValidationResult = availableAgents != null;
  const agentAvailableList = agentIdentifiers.length > 0 ? agentIdentifiers.join(', ') : 'none reported';

  const { tabs, workspaceNavItems, workspaceMetrics, contextRows, storageRows, workflowCreateHref } =
    buildInstanceSummary({
      inst,
      bpId,
      hasAgent,
      configuredAgentIdentifier,
      operatorUrl,
      ports,
      tab,
      currentPathname,
      attestationVerification,
    });

  return (
    <AnimatedPage className="mx-auto max-w-5xl px-4 sm:px-6 py-8">
      <InstanceHeader
        inst={inst}
        bpId={bpId}
        hasAgent={hasAgent}
        setSnapshotOpen={setSnapshotOpen}
        workspaceMetrics={workspaceMetrics}
      />

      <div className="mb-4 grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0 space-y-4">
          <ResourceWorkspaceNav items={workspaceNavItems} activePath={currentPathname} />
          <ResourceTabs tabs={tabs} value={tab} onValueChange={setTab} className="mb-0" />

      <InstanceAlerts
        agentConfigured={agentConfigured}
        hasAgentValidationResult={hasAgentValidationResult}
        agentIdentifierValid={agentIdentifierValid}
        configuredAgentIdentifier={configuredAgentIdentifier}
        agentAvailableList={agentAvailableList}
      />

      {/* Overview */}
      {tab === 'overview' && (
        <OverviewTab inst={inst} bpId={bpId} serviceId={serviceId} ports={ports} operatorUrl={operatorUrl} />
      )}

      {tab === 'automation' && (
        <AutomationTab inst={inst} workflowCreateHref={workflowCreateHref} hasAgent={hasAgent} />
      )}

      {tab === 'storage' && (
        <StorageTab
          rows={storageRows}
          onSnapshot={() => setSnapshotOpen(true)}
          snapshotEnabled={inst.status === 'running'}
        />
      )}

      {/* Terminal */}
      {tab === 'terminal' && (
        <TerminalTab
          isOperatorAuthed={isOperatorAuthed}
          operatorToken={operatorToken}
          sshUserDetecting={sshUserDetecting}
          operatorUrl={operatorUrl}
          terminalPath={terminalPath}
          terminalUsername={terminalUsername}
          operatorAuthError={operatorAuthError}
          handleOperatorAuthenticate={handleOperatorAuthenticate}
          isOperatorAuthenticating={isOperatorAuthenticating}
          hasWallet={hasWallet}
        />
      )}

      {/* Chat */}
      {tab === 'chat' && (
        <ChatTab
          isOperatorAuthed={isOperatorAuthed}
          agentConfigured={agentConfigured}
          agentDiscoveryLoading={agentDiscoveryLoading}
          hasAgentValidationResult={hasAgentValidationResult}
          agentIdentifierValid={agentIdentifierValid}
          configuredAgentIdentifier={configuredAgentIdentifier}
          agentAvailableList={agentAvailableList}
          inst={inst}
          operatorAuthError={operatorAuthError}
          handleOperatorAuthenticate={handleOperatorAuthenticate}
          isOperatorAuthenticating={isOperatorAuthenticating}
          hasWallet={hasWallet}
          setTab={setTab}
          agentDiscoveryError={agentDiscoveryError}
          decodedId={decodedId}
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

      {/* Secrets */}
      {tab === 'secrets' && (
        <SecretsTab
          isOperatorAuthed={isOperatorAuthed}
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
          operatorAuthError={operatorAuthError}
          handleOperatorAuthenticate={handleOperatorAuthenticate}
          isOperatorAuthenticating={isOperatorAuthenticating}
          hasWallet={hasWallet}
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
