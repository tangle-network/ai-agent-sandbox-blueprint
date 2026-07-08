import { useCreateDeploy, type DeployStatus } from '~/lib/hooks/useCreateDeploy';
import type { BlueprintDefinition, JobDefinition } from '@tangle-network/blueprint-ui';
import { truncateAddress } from '~/lib/utils/truncate-address';
import type { ConsoleTone } from './support';

export interface DeployStepProps {
  blueprint: BlueprintDefinition;
  job: JobDefinition;
  values: Record<string, unknown>;
  ports: number[];
  infra: { blueprintId: string; serviceId: string };
  entityLabel: string;
  deploy: ReturnType<typeof useCreateDeploy>;
  capacity?: number | bigint;
  provisionEstimate: bigint;
  provisionPriceFormatted: string;
  hasProvisionRfq: boolean;
  priceLoading: boolean;
  serviceInfo: { active: boolean; permitted: boolean } | null;
  serviceValidating: boolean;
  serviceError: string | null;
  validateService: (serviceId: bigint, caller?: `0x${string}`) => Promise<{
    active: boolean;
    operatorCount: number;
    owner: string;
    blueprintId: bigint | number | string;
    permitted: boolean;
    operators: `0x${string}`[];
  } | null>;
  onBack: () => void;
  onDeploy: () => void;
  onViewDetail: () => void;
  onProvisionReady: (sandboxId: string, sidecarUrl: string) => void;
}

export type DeployBlocker = {
  title: string;
  detail: string;
  icon: string;
  tone: ConsoleTone;
};

export const preflightToneClass: Record<ConsoleTone, string> = {
  brand: 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-brand)]',
  ready: 'bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)]',
  warn: 'bg-amber-400/10 text-amber-300',
  danger: 'bg-red-400/10 text-[var(--sandbox-console-danger)]',
  muted: 'bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)]',
};

export const preflightPanelClass: Record<ConsoleTone, string> = {
  brand: 'bg-[var(--sandbox-console-brand-soft)] ring-[var(--sandbox-console-brand-border)]',
  ready: 'bg-[var(--sandbox-console-success-soft)] ring-[var(--sandbox-console-success-border)]',
  warn: 'bg-amber-400/[0.08] ring-amber-400/25',
  danger: 'bg-red-400/[0.08] ring-red-400/25',
  muted: 'bg-[var(--sandbox-console-control)] ring-[var(--sandbox-console-border)]',
};

export function getServiceProblem({
  serviceInfo,
  serviceError,
  serviceId,
  blueprintId,
  address,
}: {
  serviceInfo: { active: boolean; permitted: boolean } | null;
  serviceError: string | null;
  serviceId: string;
  blueprintId: string;
  address?: string;
}): DeployBlocker | null {
  const formattedService = serviceId || '--';
  const formattedBlueprint = blueprintId || '--';

  if (serviceError) {
    return {
      title: `Service #${formattedService} not found`,
      detail: `Create a service for blueprint #${formattedBlueprint}, or choose an active service before deploying this sandbox.`,
      icon: 'i-ph:x-circle',
      tone: 'danger',
    };
  }

  if (serviceInfo && !serviceInfo.active) {
    return {
      title: `Service #${formattedService} is inactive`,
      detail: 'Choose an active service or create a replacement service before deploying this sandbox.',
      icon: 'i-ph:power',
      tone: 'warn',
    };
  }

  if (serviceInfo && !serviceInfo.permitted) {
    return {
      title: `Wallet not permitted on service #${formattedService}`,
      detail: `${address ? truncateAddress(address) : 'This wallet'} cannot submit jobs to this service. Choose or create a service where this wallet is allowed.`,
      icon: 'i-ph:lock-key',
      tone: 'danger',
    };
  }

  return null;
}

export function getDeployBlocker({
  status,
  serviceValidating,
  serviceProblem,
  contractsDeployed,
  isConnected,
  isReconnecting,
  isSandbox,
  capacity,
  isNewService,
  operatorsLoading,
  operatorCount,
  priceLoading,
}: {
  status: DeployStatus;
  serviceValidating: boolean;
  serviceProblem: DeployBlocker | null;
  contractsDeployed: boolean;
  isConnected: boolean;
  isReconnecting: boolean;
  isSandbox: boolean;
  capacity?: number | bigint;
  isNewService: boolean;
  operatorsLoading: boolean;
  operatorCount: number;
  priceLoading: boolean;
}): DeployBlocker | null {
  if (status !== 'idle') return null;
  if (serviceValidating) {
    return {
      title: 'Checking service',
      detail: 'Reading the selected service before the transaction can be built.',
      icon: 'i-ph:spinner',
      tone: 'warn',
    };
  }
  if (!contractsDeployed) {
    return {
      title: 'Contracts unavailable',
      detail: 'Switch to a supported network before deploying.',
      icon: 'i-ph:warning-circle',
      tone: 'danger',
    };
  }
  if (isReconnecting) {
    return {
      title: 'Wallet reconnecting',
      detail: 'Wait for the wallet session to finish reconnecting.',
      icon: 'i-ph:wallet',
      tone: 'warn',
    };
  }
  if (!isConnected) {
    return {
      title: 'Connect wallet',
      detail: 'A connected wallet is required to create services and submit jobs.',
      icon: 'i-ph:wallet',
      tone: 'danger',
    };
  }
  if (isSandbox && capacity !== undefined && Number(capacity) === 0) {
    return {
      title: 'No sandbox capacity',
      detail: 'All operator slots are in use. Delete unused sandboxes or try again later.',
      icon: 'i-ph:database',
      tone: 'warn',
    };
  }
  if (serviceProblem) return serviceProblem;
  if (isNewService && operatorsLoading) {
    return {
      title: 'Finding operators',
      detail: 'Operator discovery must finish before a new service request can be created.',
      icon: 'i-ph:users-three',
      tone: 'warn',
    };
  }
  if (isNewService && operatorCount === 0) {
    return {
      title: 'No operators available',
      detail: 'This blueprint needs at least one registered operator before a service can be created.',
      icon: 'i-ph:users-three',
      tone: 'danger',
    };
  }
  if (priceLoading) {
    return {
      title: 'Loading price',
      detail: 'Waiting for the operator quote before showing the deploy transaction.',
      icon: 'i-ph:receipt',
      tone: 'warn',
    };
  }

  return null;
}
