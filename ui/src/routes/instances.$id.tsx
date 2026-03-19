import { useParams, Link } from 'react-router';
import { useState, useCallback, useMemo, useEffect } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
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
import { createProxiedInstanceClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { cn } from '@tangle-network/blueprint-ui';
import { OperatorTerminalView } from '~/components/shared/OperatorTerminalView';
import { useAccount } from 'wagmi';

type ActionTab = 'overview' | 'terminal' | 'chat' | 'attestation';

export default function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const instances = useStore(instanceListStore);
  const inst = getInstance(decodedId) ?? instances.find((s) => s.id === decodedId);
  const { address } = useAccount();

  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');

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
              <LabeledValueRow
                label="Sandbox"
                value={inst.sandboxId || 'Pending operator provision'}
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
              <LabeledValueRow label="Service" value={inst.serviceId ? `#${inst.serviceId}` : 'Pending activation'} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Connection</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <LabeledValueRow label="Access" value="Operator API" />
              <LabeledValueRow label="Authenticated" value={isOperatorAuthed ? 'Yes' : 'No'} />
              {operatorAuthError && (
                <p className="text-xs text-crimson-500">{operatorAuthError}</p>
              )}
              {!isOperatorAuthed && (
                <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
                  {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
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
