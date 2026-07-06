import { type ReactNode } from 'react';
import { ConnectKitButton } from 'connectkit';
import { cn, type DiscoveredOperator } from '@tangle-network/blueprint-ui';
import { OperatorIdentity } from '~/components/shared/VisualIdentity';
import type { DeployStatus } from '~/lib/hooks/useCreateDeploy';
import { LaunchActionButton } from './launch-fields';

export function TxStatusCard({
  status, txHash, error, entityLabel, isNewService,
}: {
  status: DeployStatus;
  txHash?: `0x${string}`;
  error?: string;
  entityLabel: string;
  isNewService: boolean;
}) {
  const borderClass = status === 'confirmed' ? 'border-teal-500/20 bg-teal-500/[0.03]'
    : status === 'failed' ? 'border-crimson-500/20 bg-crimson-500/[0.03]'
    : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)]';

  const messages: Record<DeployStatus, string> = {
    idle: '',
    signing: isNewService ? 'Confirm service creation in wallet...' : 'Confirm in wallet...',
    pending: isNewService ? 'Creating service on-chain...' : 'Confirming on-chain...',
    confirmed: isNewService ? 'Service created — waiting for operator provisioning' : `${entityLabel} creation confirmed`,
    provisioning: 'Operator provisioning in progress...',
    ready: `${entityLabel} is ready`,
    failed: 'Transaction failed',
  };

  const icons: Record<DeployStatus, ReactNode> = {
    idle: null,
    signing: <div className="w-5 h-5 rounded-full border-2 border-amber-400 border-t-transparent animate-spin" />,
    pending: <div className="w-5 h-5 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />,
    confirmed: <div className="i-ph:check-circle-fill text-lg text-teal-400" />,
    provisioning: <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />,
    ready: <div className="i-ph:check-circle-fill text-lg text-teal-400" />,
    failed: <div className="i-ph:x-circle-fill text-lg text-crimson-400" />,
  };

  return (
    <div className={cn('rounded-[5px] border p-4', borderClass)}>
      <div className="flex items-center gap-3">
        {icons[status]}
        <div className="flex-1 min-w-0">
          <p className="font-display text-base font-bold text-[var(--sandbox-console-text)]">
            {messages[status]}
          </p>
          {txHash && (
            <p className="mt-1 truncate font-data text-xs text-[var(--sandbox-console-muted)]">{txHash}</p>
          )}
          {error && (
            <div className="mt-1">
              <p className="text-xs text-crimson-400">{error}</p>
              {/resource not available|request already pending/i.test(error) && (
                <p className="mt-1 text-[11px] text-[var(--sandbox-console-muted)]">
                  MetaMask may have a pending request. Open MetaMask, dismiss any popups, then try again.
                </p>
              )}
              {/user rejected|denied/i.test(error) && (
                <p className="mt-1 text-[11px] text-[var(--sandbox-console-muted)]">
                  Transaction was rejected in the wallet.
                </p>
              )}
              {/chain.*mismatch|wrong.*network/i.test(error) && (
                <p className="mt-1 text-[11px] text-[var(--sandbox-console-muted)]">
                  Your wallet is on a different network. Switch to the correct chain and try again.
                </p>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export function InstanceProvisionCard({ provision }: { provision?: { sandboxId: string; sidecarUrl: string } }) {
  return (
    <div className={cn(
      'rounded-[5px] border p-4',
      provision ? 'border-teal-500/20 bg-teal-500/[0.03]' : 'border-violet-500/20 bg-violet-500/[0.03]',
    )}>
      <div className="flex items-center gap-3">
        {provision ? (
          <>
            <div className="i-ph:check-circle-fill text-lg text-teal-400" />
            <div>
              <p className="font-display text-base font-bold text-teal-400">Instance ready</p>
              <p className="mt-1 max-w-sm truncate font-data text-xs text-[var(--sandbox-console-muted)]">
                {provision.sidecarUrl}
              </p>
            </div>
          </>
        ) : (
          <>
            <div className="w-5 h-5 rounded-full border-2 border-violet-400 border-t-transparent animate-spin" />
            <div>
              <p className="font-display text-base font-bold text-[var(--sandbox-console-text)]">Waiting for operator...</p>
              <p className="mt-1 text-xs text-[var(--sandbox-console-muted)]">Watching for on-chain provisioning event</p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export function OperatorList({
  operators,
  operatorsLoading,
  operatorsError,
  operatorCount,
  blueprintId,
  purpose = 'instance',
}: {
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  operatorsError?: Error | null;
  operatorCount: bigint;
  blueprintId: string;
  purpose?: 'instance' | 'service';
}) {
  const titleCount = operatorsLoading
    ? '...'
    : operatorsError && operatorCount > 0n
      ? operatorCount.toString()
      : String(operators.length);

  return (
    <div className="sandbox-console-panel rounded-[5px] p-4">
      <div className="flex items-center gap-2 mb-3">
        <div className="i-ph:users-three text-base text-[var(--sandbox-console-muted)]" />
        <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">
          Operators ({titleCount})
        </span>
      </div>
      {operatorsLoading ? (
        <div className="flex items-center gap-2">
          <div className="h-3 w-3 animate-spin rounded-full border border-[var(--sandbox-console-muted)] border-t-transparent" />
          <span className="text-xs text-[var(--sandbox-console-muted)]">Discovering operators for blueprint #{blueprintId}...</span>
        </div>
      ) : operatorsError ? (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <div className="i-ph:warning-circle text-sm text-amber-400" />
            <span className="text-xs text-amber-400">
              {operatorCount > 0n
                ? `Found ${operatorCount.toString()} registered operator${operatorCount === 1n ? '' : 's'} on-chain, but verification failed`
                : 'Operator lookup failed for this blueprint'}
            </span>
          </div>
          <p className="text-sm leading-6 text-[var(--sandbox-console-muted)]">
            This is usually a local RPC or multicall issue. The app could not build a verified operator list for service creation.
          </p>
        </div>
      ) : operators.length === 0 ? (
        <div className="flex items-center gap-2">
          <div className="i-ph:warning text-sm text-amber-400" />
          <span className="text-xs text-amber-400">No operators found for this blueprint</span>
        </div>
      ) : (
        <div className="space-y-1.5">
          {operators.map((op) => (
            <div key={op.address} className="flex items-center gap-2 py-1">
              <OperatorIdentity address={op.address} detail="registered operator" />
            </div>
          ))}
          <p className="mt-2 text-sm leading-6 text-[var(--sandbox-console-muted)]">
            {purpose === 'service'
              ? 'Use Create Service to request an active cloud service with these registered operators, then deploy the sandbox into that service.'
              : 'A new service will be created with these operators. Your sandbox config will be passed as service request inputs.'}
          </p>
        </div>
      )}
    </div>
  );
}

export function DeployButton({
  status, canDeploy, isNewService, priceLoading, serviceValidating, costDisplay, blockedTitle, connectWalletBlocked, onDeploy,
}: {
  status: DeployStatus;
  canDeploy: boolean;
  isNewService: boolean;
  priceLoading: boolean;
  serviceValidating: boolean;
  costDisplay: string;
  blockedTitle?: string;
  connectWalletBlocked?: boolean;
  onDeploy: () => void;
}) {
  const isBusy = status === 'signing' || status === 'pending';
  const isDisabled = !canDeploy || isBusy || priceLoading || serviceValidating;

  if (connectWalletBlocked) {
    return (
      <ConnectKitButton.Custom>
        {({ show, isConnecting }) => (
          <LaunchActionButton size="lg" className="w-full" onClick={show} disabled={isConnecting}>
            {isConnecting ? (
              <>
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/40 border-t-white" />
                Connecting wallet
              </>
            ) : (
              <>
                <div className="i-ph:plugs-connected text-base" />
                Connect wallet
              </>
            )}
          </LaunchActionButton>
        )}
      </ConnectKitButton.Custom>
    );
  }

  return (
    <LaunchActionButton size="lg" className="w-full" onClick={onDeploy} disabled={isDisabled}>
      {isBusy ? (
        <>
          <div className="w-4 h-4 rounded-full border-2 border-white/40 border-t-white animate-spin" />
          {status === 'signing' ? 'Confirm in wallet...' : 'Deploying...'}
        </>
      ) : priceLoading ? (
        'Loading price...'
      ) : blockedTitle ? (
        <>
          <div className="i-ph:lock-key text-base" />
          {blockedTitle}
        </>
      ) : isNewService ? (
        <>
          <div className="i-ph:lightning text-base" />
          Create Service & Deploy
        </>
      ) : (
        <>
          <div className="i-ph:lightning text-base" />
          Deploy for {costDisplay}
        </>
      )}
    </LaunchActionButton>
  );
}
