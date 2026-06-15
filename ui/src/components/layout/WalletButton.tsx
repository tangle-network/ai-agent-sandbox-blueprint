import { blo } from 'blo';
import { ConnectKitButton } from 'connectkit';
import { useAccount, useDisconnect } from 'wagmi';
import { useStore } from '@nanostores/react';
import { useCallback } from 'react';
import type { RefObject } from 'react';
import type { Address } from 'viem';
import { numberToHex } from 'viem';
import { networks } from '~/lib/contracts/chains';
import { cn, publicClient, selectedChainIdStore, useWalletEthBalance } from '@tangle-network/blueprint-ui';
import { useDropdownMenu } from '@tangle-network/sandbox-ui/hooks';
import { copyText } from '@tangle-network/sandbox-ui/utils';
import { truncateAddress } from '~/lib/utils/truncate-address';
import { toast } from 'sonner';
import { expectedLocalRpcUrl, walletRpcMatchesAppRpc } from '~/lib/walletRpcSync';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';

type DropdownAlign = 'start' | 'end';
type DropdownSide = 'up' | 'down';

interface WalletButtonProps {
  align?: DropdownAlign;
  side?: DropdownSide;
  compact?: boolean;
}

/**
 * Build RPC URLs suitable for wallet_addEthereumChain.
 *
 * Remote access (e.g. Tailscale): The browser can't reach localhost:8645 on the
 * server. We use the Vite dev proxy (`/rpc-proxy`) via the page origin, which
 * forwards to Anvil. MetaMask technically requires HTTPS for non-localhost URLs,
 * but many wallet versions accept plain HTTP for dev chains. We try the proxy URL
 * first; if MetaMask rejects it the toast handler guides the user.
 *
 * Local access: Use http://localhost:<port> which MetaMask always allows.
 */
function walletRpcUrls(chain: { id: number; rpcUrls: { default: { http: readonly string[] } } }): string[] {
  const envRpc = import.meta.env.VITE_RPC_URL as string | undefined;
  const localChainId = Number(import.meta.env.VITE_CHAIN_ID ?? 31337);
  if (chain.id === localChainId && envRpc) {
    const pageHost = typeof window !== 'undefined' ? window.location.hostname : 'localhost';
    const isLocalPage = pageHost === '127.0.0.1' || pageHost === 'localhost';
    if (isLocalPage) {
      // Local browser → use localhost (MetaMask's HTTP exception)
      return [envRpc.replace('127.0.0.1', 'localhost')];
    }
    // Remote browser (Tailscale/LAN) → use the Vite dev proxy through the page origin
    // so MetaMask traffic reaches Anvil on the server
    if (import.meta.env.DEV) {
      return [`${window.location.origin}/rpc-proxy`];
    }
    // Production remote: swap hostname to the page host (Anvil must be reachable)
    try {
      const rpc = new URL(envRpc);
      rpc.hostname = pageHost;
      return [rpc.toString().replace(/\/$/, '')];
    } catch {
      return [envRpc.replace('127.0.0.1', pageHost)];
    }
  }
  return [...chain.rpcUrls.default.http];
}

/** Clear wagmi's localStorage so the next connect picks a fresh wallet. */
function clearWagmiStorage() {
  try {
    for (const key of Object.keys(localStorage)) {
      if (key.startsWith('wagmi.')) localStorage.removeItem(key);
    }
  } catch {
    // SecurityError in some contexts — ignore
  }
}

function WalletIdenticon({ address, size = 28 }: { address: Address; size?: number }) {
  return (
    <span
      className="relative inline-flex shrink-0 overflow-hidden rounded-full border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <img src={blo(address)} alt="" className="h-full w-full object-cover" />
    </span>
  );
}

export function WalletButton({
  align = 'end',
  side = 'down',
  compact = false,
}: WalletButtonProps = {}) {
  const { open, ref, toggle, close } = useDropdownMenu();
  const dropdownRef = ref as RefObject<HTMLDivElement>;
  const { address, chainId, isConnected, status } = useAccount();
  const isReconnecting = status === 'reconnecting';
  const { disconnect } = useDisconnect();
  const { revokeSession } = useOperatorAuth();
  const selectedChainId = useStore(selectedChainIdStore);
  const selectedNetwork = networks[selectedChainId];
  const { balance: ethBalance } = useWalletEthBalance({
    address,
    refreshKey: selectedChainId,
    readBalance: (walletAddress) => publicClient.getBalance({ address: walletAddress as Address }),
  });

  const isWrongChain = isConnected && chainId !== selectedChainId;
  const targetChain = selectedNetwork?.chain;

  // Switch or add the target chain in the wallet.
  // MetaMask caches chains by ID — wallet_addEthereumChain won't update the
  // RPC URL if chain 31337 already exists (e.g. from Hardhat). We first try
  // removing the stale chain, then re-add with the correct RPC.
  const handleSwitchChain = useCallback(async () => {
    if (!targetChain) return;
    const provider: any = (window as any).ethereum;
    if (!provider?.request) return;
    const hexId = numberToHex(targetChain.id);
    const rpcUrls = walletRpcUrls(targetChain);
    const chainParams = {
      chainId: hexId,
      chainName: targetChain.name,
      nativeCurrency: targetChain.nativeCurrency,
      rpcUrls,
      blockExplorerUrls: targetChain.blockExplorers?.default?.url
        ? [targetChain.blockExplorers.default.url]
        : undefined,
    };

    try {
      // Try switching first
      await provider.request({
        method: 'wallet_switchEthereumChain',
        params: [{ chainId: hexId }],
      });
      // Verify that the wallet is using the same local RPC as the app. A simple
      // eth_blockNumber check is not enough when two local Anvil chains share
      // chainId 31337 but run on different ports.
      try {
        const rpcMatches = await walletRpcMatchesAppRpc(targetChain.id);
        if (rpcMatches === false) throw new Error('wallet-local-rpc-mismatch');
      } catch {
        // Chain exists but has stale RPC. Try wallet_addEthereumChain to update it
        // (MetaMask updates the RPC URL if the chain ID already exists in some versions).
        try {
          await provider.request({
            method: 'wallet_addEthereumChain',
            params: [chainParams],
          });
          const rpcMatches = await walletRpcMatchesAppRpc(targetChain.id);
          if (rpcMatches === false) {
            throw new Error('wallet-local-rpc-mismatch');
          }
        } catch {
          // wallet_addEthereumChain also failed — manual intervention needed.
          // Construct direct Anvil URL for manual setup (Anvil binds 0.0.0.0).
          const directUrl = (() => {
            try {
              const u = new URL(expectedLocalRpcUrl());
              u.hostname = window.location.hostname;
              return u.toString().replace(/\/$/, '');
            } catch { return rpcUrls[0]; }
          })();
          toast.error(
            `Switched to chain ${targetChain.id} but the wallet is still using a different local RPC. ` +
            `Open MetaMask → Settings → Networks → "${targetChain.name}" and set RPC URL to ${directUrl}`,
            { duration: 15000 },
          );
        }
      }
    } catch (err: any) {
      // 4902 = chain not found — add it fresh
      if (err?.code === 4902 || err?.data?.originalError?.code === 4902) {
        try {
          await provider.request({
            method: 'wallet_addEthereumChain',
            params: [chainParams],
          });
        } catch (addErr: any) {
          toast.error(addErr?.message ?? 'Failed to add network');
        }
      } else {
        toast.error(err?.message ?? 'Failed to switch network');
      }
    }
  }, [targetChain]);

  async function copyAddress() {
    if (!address) return;
    await copyText(address);
    toast.success('Address copied');
  }

  return (
    <ConnectKitButton.Custom>
      {({ show }) => {
        if (!isConnected) {
          return (
            <button
              type="button"
              onClick={show}
              disabled={isReconnecting}
              className={cn(
                'inline-flex h-10 max-w-full items-center justify-center gap-2 rounded-[5px] border border-[var(--sandbox-console-brand-border)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--sandbox-console-brand)_24%,var(--sandbox-console-panel-strong)),var(--sandbox-console-brand-soft))] font-display text-sm font-bold text-[var(--sandbox-console-text)] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] transition-[background-color,border-color,box-shadow,color,opacity,transform] duration-150 hover:border-[var(--sandbox-console-brand)] hover:bg-[var(--sandbox-console-brand-soft)] hover:shadow-[0_0_0_3px_rgba(168,123,255,0.13),inset_3px_0_0_var(--sandbox-console-brand)] active:scale-[0.98] disabled:cursor-wait disabled:opacity-70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60',
                compact ? 'w-10 min-w-0 px-0' : 'w-full min-w-0 px-3',
              )}
              aria-label="Connect wallet"
              title={compact ? 'Connect wallet' : undefined}
            >
              <span className="i-ph:plug-charging-bold shrink-0 text-base" aria-hidden="true" />
              {!compact ? (
                <span className="truncate">
                  {isReconnecting ? 'Reconnecting...' : 'Connect Wallet'}
                </span>
              ) : null}
            </button>
          );
        }

        const truncated = truncateAddress(address);
        const displayBalance = ethBalance ?? '...';

        return (
          <div ref={dropdownRef} className="relative min-w-0">
            <button
              type="button"
              onClick={toggle}
              className={cn(
                'group inline-flex h-10 max-w-full items-center rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color,transform] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] active:scale-[0.98] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60',
                compact ? 'w-10 justify-center px-0' : 'w-full min-w-0 justify-start gap-2.5 px-2.5',
              )}
              aria-label={`Account menu ${truncated}`}
              aria-expanded={open}
              title={compact ? truncated : undefined}
            >
              {address ? (
                <span className="relative flex h-7 w-7 shrink-0 items-center justify-center">
                  <WalletIdenticon address={address as Address} size={28} />
                  {isWrongChain ? (
                    <span className="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-amber-500 ring-2 ring-[var(--sandbox-console-control)]" title="Wrong chain" />
                  ) : null}
                </span>
              ) : null}
              {!compact ? (
                <>
                  <span className="min-w-0 flex-1 truncate text-left font-data text-sm font-bold tabular-nums">
                    {truncated}
                  </span>
                  <span className="shrink-0 font-data text-[11px] font-semibold text-[var(--sandbox-console-muted)]">
                    {displayBalance} ETH
                  </span>
                  <span className={cn('i-ph:caret-up-down shrink-0 text-xs text-[var(--sandbox-console-muted)] transition-colors group-hover:text-[var(--sandbox-console-text)]', open && 'text-[var(--sandbox-console-brand)]')} />
                </>
              ) : null}
            </button>

            {open && (
              <div
                role="menu"
                aria-label="Account actions"
                className={cn(
                  'absolute z-50 max-h-[min(28rem,calc(100vh-1rem))] w-[min(18rem,calc(100vw-1rem))] overflow-y-auto overscroll-contain rounded-[5px] border border-[var(--sandbox-console-menu-border)] bg-[var(--sandbox-console-menu)] p-3 shadow-[var(--sandbox-console-menu-shadow)]',
                  align === 'start' ? 'left-0' : 'right-0',
                  side === 'up' ? 'bottom-full mb-2' : 'top-full mt-2',
                )}
              >
                  <div className="mb-4 flex items-center gap-3">
                    {address && <WalletIdenticon address={address as Address} size={36} />}
                    <div className="min-w-0 flex-1">
                      <button onClick={copyAddress} className="flex items-center gap-2 group w-full" title="Copy address">
                        <span className="truncate font-data text-sm font-semibold text-[var(--sandbox-console-text)]">{truncated}</span>
                        <div className="i-ph:copy shrink-0 text-sm text-[var(--sandbox-console-muted)] transition-colors group-hover:text-[var(--sandbox-console-brand)]" />
                      </button>
                      <div className="font-data text-xs text-[var(--sandbox-console-muted)]">{displayBalance} ETH</div>
                    </div>
                  </div>

                  <div className="mb-3 flex items-center gap-2.5 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3 py-2.5 shadow-[var(--sandbox-console-control-shadow)]">
                    <div className={`w-2.5 h-2.5 rounded-full shrink-0 ${isWrongChain ? 'bg-amber-500 dark:bg-amber-400 animate-pulse' : 'bg-teal-600 dark:bg-teal-400'}`} />
                    <span className="flex-1 font-data text-sm text-[var(--sandbox-console-secondary)]">
                      {isWrongChain ? `Chain ${chainId}` : (targetChain?.name ?? 'Unknown')}
                    </span>
                    {isWrongChain && <span className="font-data text-xs font-semibold uppercase tracking-wider text-amber-600 dark:text-amber-400">wrong chain</span>}
                  </div>

                  <div className="space-y-1">
                    {isWrongChain && (
                      <button onClick={handleSwitchChain} className="flex w-full items-center gap-2.5 rounded-[5px] px-3 py-2.5 text-left transition-colors hover:bg-[var(--sandbox-console-brand-soft)]">
                        <div className="i-ph:swap text-base text-[var(--sandbox-console-brand)]" />
                        <span className="font-display text-sm font-semibold text-[var(--sandbox-console-secondary)]">Switch to {targetChain?.name ?? 'Unknown'}</span>
                      </button>
                    )}
                    <button onClick={copyAddress} className="flex w-full items-center gap-2.5 rounded-[5px] px-3 py-2.5 text-left transition-colors hover:bg-[var(--sandbox-console-menu-strong)]">
                      <div className="i-ph:copy text-base text-[var(--sandbox-console-muted)]" />
                      <span className="font-display text-sm font-semibold text-[var(--sandbox-console-secondary)]">Copy Address</span>
                    </button>
                    <button onClick={() => { revokeSession(); disconnect(); clearWagmiStorage(); close(); }} className="flex w-full items-center gap-2.5 rounded-[5px] px-3 py-2.5 text-left transition-colors hover:bg-crimson-500/10">
                      <div className="i-ph:sign-out text-base text-crimson-600 dark:text-crimson-400" />
                      <span className="font-display text-sm font-semibold text-crimson-600 dark:text-crimson-400">Disconnect</span>
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
