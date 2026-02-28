import { ConnectKitButton } from 'connectkit';
import { useAccount, useDisconnect, useSwitchChain } from 'wagmi';
import { useStore } from '@nanostores/react';
import type { Address } from 'viem';
import { networks } from '~/lib/contracts/chains';
import { publicClient, selectedChainIdStore } from '@tangle/blueprint-ui';
import { Identicon } from '@tangle/blueprint-ui/components';
import {
  ConnectWalletCta,
  copyText,
  truncateAddress,
  useDropdownMenu,
  useWalletEthBalance,
} from '@tangle/agent-ui/primitives';
import { toast } from 'sonner';

export function WalletButton() {
  const { open, ref, toggle, close } = useDropdownMenu();
  const { address, chainId, isConnected, status } = useAccount();
  const isReconnecting = status === 'reconnecting';
  const { disconnect } = useDisconnect();
  const { switchChain } = useSwitchChain();
  const selectedChainId = useStore(selectedChainIdStore);
  const selectedNetwork = networks[selectedChainId];
  const { balance: ethBalance } = useWalletEthBalance({
    address,
    refreshKey: selectedChainId,
    readBalance: (walletAddress) => publicClient.getBalance({ address: walletAddress as Address }),
  });

  const isWrongChain = isConnected && chainId !== selectedChainId;

  async function copyAddress() {
    if (!address) return;
    await copyText(address);
    toast.success('Address copied');
  }

  const targetChain = selectedNetwork?.chain;

  function handleSwitchChain() {
    if (targetChain) switchChain({ chainId: targetChain.id });
  }

  return (
    <ConnectKitButton.Custom>
      {({ show }) => {
        if (!isConnected) {
          return <ConnectWalletCta onClick={show} isReconnecting={isReconnecting} />;
        }

        const truncated = truncateAddress(address);
        const displayBalance = ethBalance ?? '...';

        return (
          <div ref={ref} className="relative">
            <button onClick={toggle} className="flex items-center gap-2.5 px-3 py-2 rounded-lg glass-card hover:border-violet-500/20 transition-all">
              {isWrongChain && <div className="w-2.5 h-2.5 rounded-full bg-amber-500 dark:bg-amber-400 animate-pulse shrink-0" title="Wrong chain" />}
              {address && <Identicon address={address as Address} size={22} />}
              <span className="text-sm font-data text-cloud-elements-textPrimary">{truncated}</span>
              <span className="text-xs font-data text-cloud-elements-textSecondary">{displayBalance} ETH</span>
              <div className={`i-ph:caret-down text-xs text-cloud-elements-textTertiary transition-transform ${open ? 'rotate-180' : ''}`} />
            </button>

            {open && (
              <div className="absolute right-0 top-full mt-2 w-72 glass-card-strong rounded-xl border border-cloud-elements-dividerColor/50 p-4 z-50 shadow-lg">
                <div className="flex items-center gap-3 mb-4">
                  {address && <Identicon address={address as Address} size={32} />}
                  <div className="min-w-0 flex-1">
                    <button onClick={copyAddress} className="flex items-center gap-2 group w-full" title="Copy address">
                      <span className="text-sm font-data text-cloud-elements-textPrimary truncate">{truncated}</span>
                      <div className="i-ph:copy text-sm text-cloud-elements-textTertiary group-hover:text-violet-700 dark:group-hover:text-violet-400 transition-colors shrink-0" />
                    </button>
                    <div className="text-xs font-data text-cloud-elements-textSecondary">{displayBalance} ETH</div>
                  </div>
                </div>

                <div className="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-cloud-elements-item-backgroundActive mb-3">
                  <div className={`w-2.5 h-2.5 rounded-full shrink-0 ${isWrongChain ? 'bg-amber-500 dark:bg-amber-400 animate-pulse' : 'bg-teal-600 dark:bg-teal-400'}`} />
                  <span className="text-sm font-data text-cloud-elements-textSecondary flex-1">
                    {isWrongChain ? `Chain ${chainId}` : (targetChain?.name ?? 'Unknown')}
                  </span>
                  {isWrongChain && <span className="text-xs font-data text-amber-600 dark:text-amber-400 uppercase tracking-wider font-semibold">wrong chain</span>}
                </div>

                <div className="space-y-1">
                  {isWrongChain && (
                    <button onClick={handleSwitchChain} className="flex items-center gap-2.5 w-full px-3 py-2.5 rounded-lg hover:bg-violet-500/10 transition-colors text-left">
                      <div className="i-ph:swap text-base text-violet-700 dark:text-violet-400" />
                      <span className="text-sm font-display text-cloud-elements-textSecondary">Switch to {targetChain?.name ?? 'Unknown'}</span>
                    </button>
                  )}
                  <button onClick={copyAddress} className="flex items-center gap-2.5 w-full px-3 py-2.5 rounded-lg hover:bg-cloud-elements-item-backgroundHover transition-colors text-left">
                    <div className="i-ph:copy text-base text-cloud-elements-textTertiary" />
                    <span className="text-sm font-display text-cloud-elements-textSecondary">Copy Address</span>
                  </button>
                  <button onClick={() => { disconnect(); close(); }} className="flex items-center gap-2.5 w-full px-3 py-2.5 rounded-lg hover:bg-crimson-500/10 transition-colors text-left">
                    <div className="i-ph:sign-out text-base text-crimson-600 dark:text-crimson-400" />
                    <span className="text-sm font-display text-crimson-600 dark:text-crimson-400">Disconnect</span>
                  </button>
                </div>
              </div>
            )}
          </div>
        );
      }}
    </ConnectKitButton.Custom>
  );
}
