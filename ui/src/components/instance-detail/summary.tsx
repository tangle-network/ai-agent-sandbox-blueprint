import { getBlueprint } from '@tangle-network/blueprint-ui';
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
import type { LocalInstance } from '~/lib/stores/instances';
import type { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import {
  getInstanceSandboxDisplayValue,
  getInstanceServiceDisplayValue,
  getInstanceStatusLabel,
} from '~/lib/instances/display';
import type { ActionTab } from './helpers';

interface InstanceSummaryInput {
  inst: LocalInstance;
  bpId: string;
  hasAgent: boolean;
  configuredAgentIdentifier: string;
  operatorUrl: string;
  ports: ReturnType<typeof useExposedPorts>;
  tab: ActionTab;
  currentPathname: string;
  attestationVerification: AttestationVerification | null;
}

interface InstanceTab {
  key: ActionTab;
  label: string;
  icon: string;
  hidden?: boolean;
}

interface WorkspaceNavItem {
  label: string;
  href: string;
  icon: string;
  disabled?: boolean;
}

interface InstanceSummary {
  tabs: InstanceTab[];
  workspaceNavItems: WorkspaceNavItem[];
  workspaceMetrics: ConsoleMetric[];
  contextRows: WorkspaceRailRow[];
  storageRows: WorkspaceRailRow[];
  workflowCreateHref: string | undefined;
}

export function buildInstanceSummary({
  inst,
  bpId,
  hasAgent,
  configuredAgentIdentifier,
  operatorUrl,
  ports,
  tab,
  currentPathname,
  attestationVerification,
}: InstanceSummaryInput): InstanceSummary {
  const tabs: InstanceTab[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal' },
    { key: 'chat', label: 'Chat', icon: 'i-ph:chat-circle', hidden: !hasAgent },
    { key: 'automation', label: 'Automation', icon: 'i-ph:flow-arrow' },
    { key: 'ssh' as const, label: 'SSH', icon: 'i-ph:key', hidden: !inst.sshPort },
    { key: 'secrets', label: 'Secrets', icon: 'i-ph:lock-simple', hidden: !!inst.teeEnabled },
    { key: 'storage', label: 'Storage', icon: 'i-ph:database' },
    ...(inst.teeEnabled ? [{ key: 'attestation' as const, label: 'Attestation', icon: 'i-ph:shield-check' }] : []),
  ];

  const workspaceBasePath = `/instances/${encodeURIComponent(inst.id)}`;
  const workspaceNavItems: WorkspaceNavItem[] = [
    { label: 'Runtime', href: `${workspaceBasePath}/runtime`, icon: 'i-ph:terminal' },
    { label: 'Sessions', href: `${workspaceBasePath}/sessions`, icon: 'i-ph:chat-circle', disabled: !hasAgent },
    { label: 'Automation', href: `${workspaceBasePath}/automation`, icon: 'i-ph:flow-arrow' },
    { label: 'Network', href: `${workspaceBasePath}/network`, icon: 'i-ph:plugs', disabled: !inst.sshPort },
    { label: 'Security', href: `${workspaceBasePath}/security`, icon: 'i-ph:shield-check' },
    { label: 'Storage', href: `${workspaceBasePath}/storage`, icon: 'i-ph:database' },
  ];
  const exposedPortCount = ports?.length ?? 0;
  const statusTone = inst.status === 'running' ? 'ready' : inst.status === 'creating' ? 'brand' : inst.status === 'error' ? 'danger' : 'muted';
  const workspaceMetrics: ConsoleMetric[] = [
    {
      label: 'Status',
      value: getInstanceStatusLabel(inst),
      detail: inst.status === 'running' ? 'operator ready' : 'lifecycle',
      tone: statusTone,
      identity: getStatusIdentity(inst.status),
    },
    {
      label: 'Runtime',
      value: inst.teeEnabled ? 'TEE' : 'Docker',
      detail: `${inst.cpuCores}c / ${Math.round(inst.memoryMb / 1024)}g / ${inst.diskGb}g`,
      tone: inst.teeEnabled ? 'warn' : 'brand',
      identity: getRuntimeIdentity(inst.teeEnabled ? 'tee' : 'docker'),
    },
    {
      label: 'Network',
      value: inst.sshPort ? `ssh:${inst.sshPort}` : exposedPortCount > 0 ? `${exposedPortCount} ports` : 'proxy',
      detail: operatorUrl.replace(/^https?:\/\//, ''),
      tone: inst.sshPort || exposedPortCount > 0 ? 'ready' : 'muted',
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
    { label: 'Instance ID', value: inst.id, detail: getInstanceSandboxDisplayValue(inst), tone: 'brand', identity: getBlueprintIdentity(bpId) },
    { label: 'Blueprint', value: getBlueprint(bpId)?.name ?? bpId, detail: getInstanceServiceDisplayValue(inst), tone: 'brand', identity: getBlueprintIdentity(bpId) },
    { label: 'Operator', value: inst.operator ? truncateAddress(inst.operator) : 'unknown', detail: inst.operator ?? 'operator not resolved', tone: inst.operator ? 'ready' : 'muted', leading: inst.operator ? <OperatorIdenticon address={inst.operator} size="sm" /> : undefined },
    { label: 'Workspace', value: tab, detail: currentPathname, tone: 'muted', identity: getStatusIdentity('processing') },
  ];
  // Secrets trust signal comes from the real server attestation verdict, not the
  // `teeEnabled` config flag: "Attested / hardware trust" only once the verdict
  // is `verified`, otherwise "TEE requested — unverified" with no shield-check.
  const secretsTeeVerified = Boolean(inst.teeEnabled) && isAttestationVerified(attestationVerification);
  const secretsRowIdentity = getTeeSecurityIdentity({
    credentialsMissing: inst.credentialsAvailable === false,
    teeRequested: Boolean(inst.teeEnabled),
    verified: secretsTeeVerified,
  });
  const secretsRowDetail = inst.teeEnabled
    ? secretsTeeVerified
      ? 'TEE attested'
      : 'TEE requested — unverified'
    : 'operator encrypted';
  const storageRows: WorkspaceRailRow[] = [
    { label: 'Image', value: inst.image.replace('ghcr.io/tangle-network/', ''), detail: 'source image', tone: 'brand', identity: getImageIdentity(inst.image) },
    { label: 'Disk', value: `${inst.diskGb} GB`, detail: 'allocated volume', tone: 'ready', identity: getResourceIdentity('disk') },
    { label: 'Lifecycle', value: inst.status, detail: new Date(inst.createdAt).toLocaleString(), tone: statusTone, identity: getStatusIdentity(inst.status) },
    { label: 'Secrets', value: inst.credentialsAvailable === false ? 'missing' : 'available', detail: secretsRowDetail, tone: inst.credentialsAvailable === false ? 'warn' : 'ready', identity: secretsRowIdentity },
  ];
  const workflowCreateHref = inst.status === 'running' && inst.serviceId
    ? `/workflows/create?target=${encodeURIComponent(`instance:${inst.id}`)}`
    : undefined;

  return { tabs, workspaceNavItems, workspaceMetrics, contextRows, storageRows, workflowCreateHref };
}
