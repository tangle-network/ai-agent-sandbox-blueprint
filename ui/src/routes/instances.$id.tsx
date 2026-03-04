import { useParams, Link } from 'react-router';
import { lazy, Suspense, useState, useCallback, useMemo, useEffect } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle } from '@tangle/blueprint-ui/components';
import { Button } from '@tangle/blueprint-ui/components';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import { ResourceIdentity } from '~/components/shared/ResourceIdentity';
import { LabeledValueRow } from '~/components/shared/LabeledValueRow';
import { ExposedPortsCard } from '~/components/shared/ExposedPortsCard';
import { TeeAttestationCard } from '~/components/shared/TeeAttestationCard';
import { SidecarAuthPrompt } from '~/components/shared/SidecarAuthPrompt';
import { ResourceTabs } from '~/components/shared/ResourceTabs';
import { instanceListStore, updateInstanceStatus } from '~/lib/stores/instances';
import { getBlueprint } from '@tangle/blueprint-ui';
import { useWagmiSidecarAuth } from '~/lib/hooks/useWagmiSidecarAuth';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { useOperatorApiCall } from '~/lib/hooks/useOperatorApiCall';
import { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { useTeeAttestation } from '~/lib/hooks/useTeeAttestation';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { createDirectClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { cn } from '@tangle/blueprint-ui';

const TerminalView = lazy(() =>
  import('@tangle-network/agent-ui/terminal').then((m) => ({ default: m.TerminalView })),
);

type ActionTab = 'overview' | 'terminal' | 'chat' | 'attestation';

export default function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const instances = useStore(instanceListStore);
  const inst = instances.find((s) => s.id === decodedId);

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
  const buildPath = useCallback((action: string) => `/api/sandbox/${action}`, []);
  const operatorApiCall = useOperatorApiCall(operatorUrl, getOperatorToken, buildPath);

  const ports = useExposedPorts(inst?.status, operatorApiCall);

  const {
    attestation,
    busy: attestationBusy,
    error: attestationError,
    fetchAttestation: handleFetchAttestation,
  } = useTeeAttestation(operatorApiCall);

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
            teeEnabled={inst.teeEnabled}
            image={inst.image}
            specs={`${inst.cpuCores} CPU · ${inst.memoryMb}MB · ${inst.diskGb}GB`}
            titleClassName="text-xl"
            teeStyle="pill"
          />
        </div>
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
              <LabeledValueRow label="Image" value={inst.image} mono copyable />
              <LabeledValueRow label="CPU" value={`${inst.cpuCores} cores`} />
              <LabeledValueRow label="Memory" value={`${inst.memoryMb} MB`} />
              <LabeledValueRow label="Disk" value={`${inst.diskGb} GB`} />
              <LabeledValueRow label="Created" value={new Date(inst.createdAt).toLocaleString()} />
              <LabeledValueRow label="Blueprint" value={getBlueprint(bpId)?.name ?? bpId} />
              <LabeledValueRow label="Service" value={`#${inst.serviceId}`} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Connection</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {inst.sidecarUrl ? (
                <LabeledValueRow label="Sidecar URL" value={inst.sidecarUrl} mono copyable />
              ) : (
                <div className="flex justify-between items-center text-sm">
                  <span className="text-cloud-elements-textSecondary">Sidecar URL</span>
                  <span className="flex items-center gap-2 text-xs font-data text-violet-400">
                    <div className="i-ph:circle-fill text-[8px] animate-pulse" />
                    Provisioning...
                  </span>
                </div>
              )}
              <LabeledValueRow label="Authenticated" value={isSidecarAuthed ? 'Yes' : 'No'} />
              {!isSidecarAuthed && inst.sidecarUrl && (
                <Button size="sm" onClick={sidecarAuth} disabled={isAuthenticating}>
                  {isAuthenticating ? 'Authenticating...' : 'Authenticate'}
                </Button>
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
            {isSidecarAuthed && sidecarUrl ? (
              <Suspense fallback={<div className="p-6 text-sm text-cloud-elements-textTertiary">Loading terminal...</div>}>
                <div className="h-[min(500px,60vh)]">
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
            <div className="h-[min(600px,65vh)]">
              {!isSidecarAuthed ? (
                <SidecarAuthPrompt
                  message="Authenticate to start chatting"
                  actionLabel="Authenticate"
                  busyLabel="Authenticating..."
                  isBusy={isAuthenticating}
                  isWaiting={!sidecarUrl}
                  onAuthenticate={sidecarAuth}
                />
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
          <TeeAttestationCard
            subjectLabel="instance"
            attestation={attestation}
            busy={attestationBusy}
            error={attestationError}
            onFetch={handleFetchAttestation}
          />
        </div>
      )}
    </AnimatedPage>
  );
}
