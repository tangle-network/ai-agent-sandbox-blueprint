import { useParams, Link } from 'react-router';
import { lazy, Suspense, useState, useCallback, useMemo, useEffect } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle/blueprint-ui/components';
import { Button } from '@tangle/blueprint-ui/components';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import { instanceListStore, updateInstanceStatus } from '~/lib/stores/instances';
import { useSubmitJob } from '@tangle/blueprint-ui';
import { encodeJobArgs } from '@tangle/blueprint-ui';
import { getBlueprint, getJobById } from '~/lib/blueprints';
import { INSTANCE_JOB_IDS, INSTANCE_PRICING_TIERS } from '~/lib/types/instance';
import { useWagmiSidecarAuth } from '~/lib/hooks/useWagmiSidecarAuth';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { createDirectClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { cn } from '@tangle/blueprint-ui';
import { bytesToHex, type AttestationData } from '~/lib/tee';

const TerminalView = lazy(() =>
  import('@tangle/agent-ui/terminal').then((m) => ({ default: m.TerminalView })),
);

type ActionTab = 'overview' | 'terminal' | 'chat' | 'attestation';

export default function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const instances = useStore(instanceListStore);
  const inst = instances.find((s) => s.id === decodedId);

  const { submitJob, status: txStatus } = useSubmitJob();
  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');

  const serviceId = BigInt(inst?.serviceId ?? '1');
  const bpId = inst?.teeEnabled ? 'ai-agent-tee-instance-blueprint' : 'ai-agent-instance-blueprint';
  const isCreating = inst?.status === 'creating' && !inst?.sidecarUrl;

  // Watch for OperatorProvisioned event if instance is still creating
  const instanceProvision = useInstanceProvisionWatcher(
    serviceId,
    inst?.teeEnabled ? 'tee-instance' : 'instance',
    isCreating,
  );

  useEffect(() => {
    if (instanceProvision && decodedId) {
      updateInstanceStatus(decodedId, 'running', {
        id: instanceProvision.sandboxId,
        sidecarUrl: instanceProvision.sidecarUrl,
      });
    }
  }, [instanceProvision, decodedId]);

  // Sidecar auth
  const sidecarUrl = inst?.sidecarUrl ?? '';
  const { token: sidecarToken, isAuthenticated: isSidecarAuthed, authenticate: sidecarAuth, isAuthenticating } =
    useWagmiSidecarAuth(decodedId, sidecarUrl);

  const client: SandboxClient | null = useMemo(() => {
    if (!sidecarUrl || !sidecarToken) return null;
    return createDirectClient(sidecarUrl, sidecarToken);
  }, [sidecarUrl, sidecarToken]);

  // Operator API auth for attestation
  const operatorUrl = INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL;
  const { getToken: getOperatorToken } = useOperatorAuth(operatorUrl);

  /** Call operator API for instance operations (attestation). */
  const operatorApiCall = useCallback(async (
    action: string,
    body?: Record<string, unknown>,
    opts?: { method?: string },
  ) => {
    const token = await getOperatorToken();
    if (!token) throw new Error('Wallet authentication required');

    const url = `${operatorUrl}/api/sandbox/${action}`;

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
  }, [operatorUrl, getOperatorToken]);

  // Ports state
  const [ports, setPorts] = useState<{ container_port: number; host_port: number; protocol: string }[] | null>(null);

  // Fetch exposed ports when instance is running
  useEffect(() => {
    if (inst?.status !== 'running' && inst?.status !== 'creating') return;
    let cancelled = false;
    operatorApiCall('ports', undefined, { method: 'GET' })
      .then((res) => res.json())
      .then((data) => {
        if (!cancelled && Array.isArray(data)) setPorts(data);
      })
      .catch(() => { /* ports endpoint may not exist — ignore */ });
    return () => { cancelled = true; };
  }, [inst?.status, operatorApiCall]);

  // Attestation state
  const [attestation, setAttestation] = useState<AttestationData | null>(null);
  const [attestationBusy, setAttestationBusy] = useState(false);
  const [attestationError, setAttestationError] = useState<string | null>(null);

  const handleFetchAttestation = useCallback(async () => {
    setAttestationBusy(true);
    setAttestationError(null);
    try {
      const res = await operatorApiCall('tee/attestation', undefined, { method: 'GET' });
      const data: AttestationData = await res.json();
      setAttestation(data);
    } catch (e) {
      setAttestationError(e instanceof Error ? e.message : 'Failed to fetch attestation');
    } finally {
      setAttestationBusy(false);
    }
  }, [operatorApiCall]);

  /** Compute job value from pricing tier (base rate = 0.001 TNT = 1e15 wei) */
  const jobValue = (jobId: number): bigint =>
    BigInt(INSTANCE_PRICING_TIERS[jobId]?.multiplier ?? 1) * 1_000_000_000_000_000n;

  const handleDeprovision = useCallback(async () => {
    const job = getJobById(bpId, INSTANCE_JOB_IDS.DEPROVISION);
    if (!job) return;
    const args = encodeJobArgs(job, { json: '{}' });
    const hash = await submitJob({
      serviceId,
      jobId: INSTANCE_JOB_IDS.DEPROVISION,
      args,
      label: 'Deprovision Instance',
      value: jobValue(INSTANCE_JOB_IDS.DEPROVISION),
    });
    if (hash) updateInstanceStatus(decodedId, 'gone');
  }, [bpId, serviceId, decodedId, submitJob]);

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

  const tabs: { key: ActionTab; label: string; icon: string }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal' },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle' },
    ...(inst.teeEnabled ? [{ key: 'attestation' as const, label: 'Attestation', icon: 'i-ph:shield-check' }] : []),
  ];

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-4">
          <Link to="/instances">
            <Button variant="ghost" size="sm">
              <div className="i-ph:arrow-left text-base" />
            </Button>
          </Link>
          <div>
            <div className="flex items-center gap-3">
              <h1 className="text-xl font-display font-bold text-cloud-elements-textPrimary">{inst.name}</h1>
              <StatusBadge status={inst.status === 'creating' ? 'running' : inst.status} />
              {inst.teeEnabled && (
                <span className="text-xs text-violet-700 dark:text-violet-400 font-data bg-violet-500/10 px-2 py-0.5 rounded-full">TEE</span>
              )}
            </div>
            <p className="text-xs font-data text-cloud-elements-textTertiary mt-1">
              {inst.image} · {inst.cpuCores} CPU · {inst.memoryMb}MB
            </p>
          </div>
        </div>
        <Button variant="destructive" size="sm" onClick={handleDeprovision} disabled={txStatus !== 'idle'}>
          <div className="i-ph:trash text-sm" />
          Deprovision
          <JobPriceBadge jobIndex={INSTANCE_JOB_IDS.DEPROVISION} pricingMultiplier={INSTANCE_PRICING_TIERS[INSTANCE_JOB_IDS.DEPROVISION]?.multiplier ?? 1} compact />
        </Button>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 mb-6 border-b border-cloud-elements-dividerColor">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={cn(
              'flex items-center gap-2 px-4 py-2.5 text-sm font-display font-medium transition-colors border-b-2 -mb-px',
              tab === t.key
                ? 'border-violet-500 text-cloud-elements-textPrimary'
                : 'border-transparent text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary',
            )}
          >
            <div className={`${t.icon} text-base`} />
            {t.label}
          </button>
        ))}
      </div>

      {/* Overview */}
      {tab === 'overview' && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <Card>
            <CardHeader>
              <CardTitle>Instance Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <Row label="ID" value={inst.id} mono />
              <Row label="Image" value={inst.image} mono />
              <Row label="CPU" value={`${inst.cpuCores} cores`} />
              <Row label="Memory" value={`${inst.memoryMb} MB`} />
              <Row label="Disk" value={`${inst.diskGb} GB`} />
              <Row label="Created" value={new Date(inst.createdAt).toLocaleString()} />
              <Row label="Blueprint" value={getBlueprint(bpId)?.name ?? bpId} />
              <Row label="Service" value={`#${inst.serviceId}`} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Connection</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {inst.sidecarUrl ? (
                <Row label="Sidecar URL" value={inst.sidecarUrl} mono />
              ) : (
                <div className="flex justify-between items-center text-sm">
                  <span className="text-cloud-elements-textSecondary">Sidecar URL</span>
                  <span className="flex items-center gap-2 text-xs font-data text-violet-400">
                    <div className="i-ph:circle-fill text-[8px] animate-pulse" />
                    Provisioning...
                  </span>
                </div>
              )}
              <Row label="Authenticated" value={isSidecarAuthed ? 'Yes' : 'No'} />
              {!isSidecarAuthed && inst.sidecarUrl && (
                <Button size="sm" onClick={sidecarAuth} disabled={isAuthenticating}>
                  {isAuthenticating ? 'Authenticating...' : 'Authenticate'}
                </Button>
              )}
            </CardContent>
          </Card>

          {/* Exposed Ports */}
          {ports && ports.length > 0 && (
            <Card className="lg:col-span-2">
              <CardHeader>
                <CardTitle className="text-sm">Exposed Ports</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
                  {ports.map((p) => (
                    <div
                      key={p.container_port}
                      className="flex items-center gap-2 px-3 py-2 rounded-lg bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor"
                    >
                      <div className="i-ph:globe text-sm text-teal-400" />
                      <div className="min-w-0">
                        <span className="text-xs font-data font-medium text-cloud-elements-textPrimary">
                          :{p.container_port}
                        </span>
                        <span className="text-[10px] text-cloud-elements-textTertiary ml-1.5">
                          {p.protocol}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
                <p className="text-[11px] text-cloud-elements-textTertiary mt-2">
                  Access via <span className="font-data">/api/sandbox/port/{'{port}'}/</span>
                </p>
              </CardContent>
            </Card>
          )}
        </div>
      )}

      {/* Terminal */}
      {tab === 'terminal' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0">
            {isSidecarAuthed && sidecarUrl ? (
              <Suspense fallback={<div className="p-6 text-sm text-cloud-elements-textTertiary">Loading terminal...</div>}>
                <div className="h-[500px]">
                  <TerminalView
                    apiUrl={sidecarUrl}
                    token={sidecarToken!}
                    title="Instance Terminal"
                    subtitle="Connected to sidecar PTY session"
                  />
                </div>
              </Suspense>
            ) : (
              <div className="p-6 text-center">
                <p className="text-sm text-cloud-elements-textSecondary mb-3">
                  Authenticate to access the terminal
                </p>
                <Button size="sm" onClick={sidecarAuth} disabled={isAuthenticating || !sidecarUrl}>
                  {isAuthenticating ? 'Authenticating...' : 'Authenticate'}
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
            <div className="h-[600px]">
              {!isSidecarAuthed ? (
                <div className="flex flex-col items-center justify-center h-full gap-3">
                  <div className="i-ph:chat-circle text-3xl text-cloud-elements-textTertiary" />
                  <p className="text-sm text-cloud-elements-textSecondary">
                    Authenticate to start chatting
                  </p>
                  <Button size="sm" onClick={sidecarAuth} disabled={isAuthenticating || !sidecarUrl}>
                    {isAuthenticating ? 'Authenticating...' : 'Authenticate'}
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

      {/* Attestation Tab — TEE attestation verification */}
      {tab === 'attestation' && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">TEE Attestation</CardTitle>
              <CardDescription>Verify the Trusted Execution Environment attestation for this instance</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <Button
                size="sm"
                onClick={handleFetchAttestation}
                disabled={attestationBusy}
              >
                <div className="i-ph:shield-check text-sm" />
                {attestationBusy ? 'Fetching...' : attestation ? 'Refresh Attestation' : 'Get Attestation'}
              </Button>

              {attestationError && (
                <p className="text-xs text-red-400">{attestationError}</p>
              )}

              {attestation && (
                <div className="space-y-3">
                  <Row label="TEE Type" value={attestation.tee_type} />
                  <Row
                    label="Timestamp"
                    value={new Date(attestation.timestamp * 1000).toLocaleString()}
                  />
                  <div className="space-y-1.5">
                    <span className="text-sm text-cloud-elements-textSecondary">Measurement</span>
                    <div className="p-3 rounded-lg bg-cloud-elements-background-depth-2">
                      <code className="text-xs font-data text-cloud-elements-textPrimary break-all">
                        {bytesToHex(attestation.measurement)}
                      </code>
                    </div>
                  </div>
                  <details className="group">
                    <summary className="text-sm text-cloud-elements-textSecondary cursor-pointer hover:text-cloud-elements-textPrimary transition-colors">
                      Evidence ({attestation.evidence.length} bytes)
                    </summary>
                    <div className="mt-2 p-3 rounded-lg bg-cloud-elements-background-depth-2 max-h-48 overflow-y-auto">
                      <code className="text-xs font-data text-cloud-elements-textTertiary break-all">
                        {bytesToHex(attestation.evidence)}
                      </code>
                    </div>
                  </details>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}
    </AnimatedPage>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between text-sm">
      <span className="text-cloud-elements-textSecondary">{label}</span>
      <span className={cn('text-cloud-elements-textPrimary', mono && 'font-data text-xs')}>{value}</span>
    </div>
  );
}
