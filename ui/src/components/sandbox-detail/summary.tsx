import type { ConsoleMetric } from '~/components/console/ConsolePrimitives';
import type { WorkspaceRailRow } from '~/components/console/ResourceWorkspacePanels';
import {
  OperatorIdenticon,
  getAgentIdentity,
  getBlueprintIdentity,
  getImageIdentity,
  getResourceIdentity,
  getRuntimeIdentity,
  getStatusIdentity,
  getTeeSecurityIdentity,
} from '~/components/shared/VisualIdentity';
import { truncateAddress } from '~/lib/utils/truncate-address';
import { isAttestationVerified } from '~/lib/tee';
import type { AttestationVerification } from '~/lib/tee';
import type { LocalSandbox } from '~/lib/stores/sandboxes';
import type { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { formatBlueprintLabel, formatServiceId } from './helpers';
import type { ActionTab } from './helpers';

interface SandboxSummaryInput {
  sb: LocalSandbox;
  isRunning: boolean;
  isCreating: boolean;
  isStopped: boolean;
  isGone: boolean;
  hasProvisionedSandbox: boolean;
  hasAgent: boolean;
  configuredAgentIdentifier: string;
  operatorUrl: string;
  ports: ReturnType<typeof useExposedPorts>;
  tab: ActionTab;
  currentPathname: string;
  routeKey: string;
  attestationVerification: AttestationVerification | null;
}

interface SandboxTab {
  key: ActionTab;
  label: string;
  icon: string;
  disabled?: boolean;
  hidden?: boolean;
}

interface WorkspaceNavItem {
  label: string;
  href: string;
  icon: string;
  disabled?: boolean;
}

interface SandboxSummary {
  tabs: SandboxTab[];
  workspaceNavItems: WorkspaceNavItem[];
  workspaceMetrics: ConsoleMetric[];
  contextRows: WorkspaceRailRow[];
  storageRows: WorkspaceRailRow[];
  workflowCreateHref: string | undefined;
}

export function buildSandboxSummary({
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
}: SandboxSummaryInput): SandboxSummary {
  const tabs: SandboxTab[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle', disabled: !hasProvisionedSandbox || !isRunning, hidden: !hasAgent },
    { key: 'automation', label: 'Automation', icon: 'i-ph:flow-arrow', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'ssh', label: 'SSH', icon: 'i-ph:key', disabled: !hasProvisionedSandbox || !isRunning, hidden: !sb.sshPort },
    { key: 'secrets', label: 'Secrets', icon: 'i-ph:lock-simple', disabled: !hasProvisionedSandbox || !isRunning },
    { key: 'storage', label: 'Storage', icon: 'i-ph:database', disabled: !hasProvisionedSandbox },
    { key: 'attestation', label: 'Attestation', icon: 'i-ph:shield-check', hidden: !hasProvisionedSandbox || !sb.teeEnabled },
  ];

  const workspaceBasePath = `/sandboxes/${encodeURIComponent(routeKey)}`;
  const workspaceNavItems: WorkspaceNavItem[] = [
    { label: 'Runtime', href: `${workspaceBasePath}/runtime`, icon: 'i-ph:terminal' },
    { label: 'Sessions', href: `${workspaceBasePath}/sessions`, icon: 'i-ph:chat-circle', disabled: !hasAgent },
    { label: 'Automation', href: `${workspaceBasePath}/automation`, icon: 'i-ph:flow-arrow' },
    { label: 'Network', href: `${workspaceBasePath}/network`, icon: 'i-ph:plugs', disabled: !sb.sshPort },
    { label: 'Security', href: `${workspaceBasePath}/security`, icon: 'i-ph:shield-check' },
    { label: 'Storage', href: `${workspaceBasePath}/storage`, icon: 'i-ph:database' },
  ];
  const exposedPortCount = ports?.length ?? 0;
  const statusTone = isRunning ? 'ready' : isCreating ? 'brand' : isStopped ? 'warn' : sb.status === 'error' ? 'danger' : 'muted';
  const workspaceMetrics: ConsoleMetric[] = [
    {
      label: 'Status',
      value: sb.status,
      detail: isRunning ? 'operator ready' : isCreating ? 'provisioning' : 'lifecycle',
      tone: statusTone,
      identity: getStatusIdentity(sb.status),
    },
    {
      label: 'Runtime',
      value: sb.teeEnabled ? 'TEE' : 'Docker',
      detail: `${sb.cpuCores}c / ${Math.round(sb.memoryMb / 1024)}g / ${sb.diskGb}g`,
      tone: sb.teeEnabled ? 'warn' : 'brand',
      identity: getRuntimeIdentity(sb.teeEnabled ? 'tee' : 'docker'),
    },
    {
      label: 'Network',
      value: sb.sshPort ? `ssh:${sb.sshPort}` : exposedPortCount > 0 ? `${exposedPortCount} ports` : 'proxy',
      detail: operatorUrl.replace(/^https?:\/\//, ''),
      tone: sb.sshPort || exposedPortCount > 0 ? 'ready' : 'muted',
      identity: getResourceIdentity('network'),
    },
    {
      label: 'Agent',
      value: configuredAgentIdentifier || 'none',
      detail: hasAgent ? 'sessions enabled' : 'compute only',
      tone: hasAgent ? 'brand' : 'muted',
      identity: getAgentIdentity(configuredAgentIdentifier),
    },
  ];
  const contextRows: WorkspaceRailRow[] = [
    { label: 'Sandbox ID', value: sb.sandboxId ? 'provisioned' : 'pending', detail: sb.sandboxId ?? sb.localId, tone: sb.sandboxId ? 'ready' : 'warn', identity: getStatusIdentity(sb.sandboxId ? 'running' : 'creating') },
    { label: 'Blueprint', value: formatBlueprintLabel(sb.blueprintId), detail: `service ${formatServiceId(sb.serviceId)}`, tone: 'brand', identity: getBlueprintIdentity(sb.blueprintId) },
    { label: 'Operator', value: sb.operator ? truncateAddress(sb.operator) : 'unknown', detail: sb.operator ?? 'operator not resolved', tone: sb.operator ? 'ready' : 'muted', leading: sb.operator ? <OperatorIdenticon address={sb.operator} size="sm" /> : undefined },
    { label: 'Workspace', value: tab, detail: currentPathname, tone: 'muted', identity: getStatusIdentity('processing') },
  ];
  // Drive the Secrets row's trust signal from the REAL server attestation
  // verdict, not the `teeEnabled` config flag. A TEE-requested sandbox is only
  // shown as "Attested / hardware trust" once the server verdict is `verified`;
  // otherwise it reads "TEE requested — unverified" with no shield-check.
  const secretsTeeVerified = Boolean(sb.teeEnabled) && isAttestationVerified(attestationVerification);
  const secretsRowIdentity = getTeeSecurityIdentity({
    credentialsMissing: sb.credentialsAvailable === false,
    teeRequested: Boolean(sb.teeEnabled),
    verified: secretsTeeVerified,
  });
  const secretsRowDetail = sb.teeEnabled
    ? secretsTeeVerified
      ? 'TEE attested'
      : 'TEE requested — unverified'
    : 'operator encrypted';
  const storageRows: WorkspaceRailRow[] = [
    { label: 'Image', value: sb.image.replace('ghcr.io/tangle-network/', ''), detail: 'source image', tone: 'brand', identity: getImageIdentity(sb.image) },
    { label: 'Disk', value: `${sb.diskGb} GB`, detail: 'allocated volume', tone: 'ready', identity: getResourceIdentity('disk') },
    { label: 'Lifecycle', value: isStopped ? 'warm' : isGone ? 'gone' : sb.status, detail: sb.lastActivityAt ? new Date(sb.lastActivityAt).toLocaleString() : 'no activity timestamp', tone: statusTone, identity: getStatusIdentity(isStopped ? 'stopped' : isGone ? 'error' : sb.status) },
    { label: 'Secrets', value: sb.credentialsAvailable === false ? 'missing' : 'available', detail: secretsRowDetail, tone: sb.credentialsAvailable === false ? 'warn' : 'ready', identity: secretsRowIdentity },
  ];
  const workflowCreateHref = isRunning && sb.sandboxId
    ? `/workflows/create?target=${encodeURIComponent(`sandbox:${sb.sandboxId}`)}`
    : undefined;

  return { tabs, workspaceNavItems, workspaceMetrics, contextRows, storageRows, workflowCreateHref };
}
