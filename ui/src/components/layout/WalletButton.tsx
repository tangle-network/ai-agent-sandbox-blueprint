import { ConnectKitButton } from 'connectkit';
import { useAccount, useDisconnect } from 'wagmi';
import { useStore } from '@nanostores/react';
import { useCallback } from 'react';
import type { RefObject } from 'react';
import type { Address } from 'viem';
import { numberToHex } from 'viem';
import { networks } from '~/lib/contracts/chains';
import { publicClient, selectedChainIdStore, useWalletEthBalance } from '@tangle-network/blueprint-ui';
import { ConnectWalletCta } from '@tangle-network/blueprint-ui/components';
import { useDropdownMenu } from '@tangle-network/sandbox-ui/hooks';
import { copyText } from '@tangle-network/sandbox-ui/utils';
import { truncateAddress } from '~/lib/utils/truncate-address';
import { toast } from 'sonner';
import { expectedLocalRpcUrl, walletRpcMatchesAppRpc } from '~/lib/walletRpcSync';
import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { TangleOperatorMark } from '~/components/shared/TangleBrand';

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

export function WalletButton() {
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
          return <ConnectWalletCta onClick={show} isReconnecting={isReconnecting} />;
        }

        const truncated = truncateAddress(address);
        const displayBalance = ethBalance ?? '...';

        return (
          <div className="flex items-center gap-2">
            {/* Wrong-chain banner — always visible, prominent */}
            {isWrongChain && (
              <button
                onClick={handleSwitchChain}
                className="flex h-10 items-center gap-2 rounded-[5px] border border-amber-500/35 bg-amber-500/12 px-3 font-display text-sm font-semibold text-amber-700 transition-colors hover:bg-amber-500/20 dark:text-amber-300"
              >
                <div className="w-2 h-2 rounded-full bg-amber-500 animate-pulse shrink-0" />
                <span>
                  Switch to {targetChain?.name ?? `Chain ${selectedChainId}`}
                </span>
              </button>
            )}

            <div ref={dropdownRef} className="relative">
              <button
                onClick={toggle}
                className="flex h-10 items-center gap-2.5 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3 text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]"
              >
                {address && <TangleOperatorMark label={address} />}
                <span className="font-data text-sm font-semibold">{truncated}</span>
                <span className="font-data text-xs text-[var(--sandbox-console-muted)]">{displayBalance} ETH</span>
                <div className={`i-ph:caret-down text-xs text-[var(--sandbox-console-muted)] transition-transform ${open ? 'rotate-180 text-[var(--sandbox-console-brand)]' : ''}`} />
              </button>

              {open && (
                <div className="absolute right-0 top-full z-50 mt-2 w-72 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel-strong)] p-4 shadow-[var(--sandbox-console-shadow-lg)]">
                  <div className="flex items-center gap-3 mb-4">
                    {address && <TangleOperatorMark label={address} className="h-9 w-9 p-1.5" />}
                    <div className="min-w-0 flex-1">
                      <button onClick={copyAddress} className="flex items-center gap-2 group w-full" title="Copy address">
                        <span className="truncate font-data text-sm font-semibold text-[var(--sandbox-console-text)]">{truncated}</span>
                        <div className="i-ph:copy shrink-0 text-sm text-[var(--sandbox-console-muted)] transition-colors group-hover:text-[var(--sandbox-console-brand)]" />
                      </button>
                      <div className="font-data text-xs text-[var(--sandbox-console-muted)]">{displayBalance} ETH</div>
                    </div>
                  </div>

                  <div className="mb-3 flex items-center gap-2.5 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3 py-2.5">
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
                    <button onClick={copyAddress} className="flex w-full items-center gap-2.5 rounded-[5px] px-3 py-2.5 text-left transition-colors hover:bg-[var(--sandbox-console-control-hover)]">
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
          </div>
        );
      }}
    </ConnectKitButton.Custom>
  );
}
