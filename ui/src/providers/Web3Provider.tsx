import { createConfig, http, type Config } from 'wagmi';
import { ConnectKitProvider, getDefaultConfig } from 'connectkit';
import { type ReactNode } from 'react';
import type { Chain } from 'viem';
import {
  defaultConnectKitOptions,
} from '@tangle-network/blueprint-ui';
import { Web3Shell } from '@tangle-network/blueprint-ui/components';
import {
  detectTangleCloudParentOrigin,
  parentBridgeConnector,
} from '@tangle-network/blueprint-ui/wallet';
import { baseSepolia, networks } from '~/lib/contracts/chains';

const appMetadata = {
  appName: 'Tangle Sandbox Cloud',
  appDescription: 'AI Agent Sandbox Provisioning on Tangle Network',
  appUrl: typeof window !== 'undefined' ? window.location.origin : 'https://cloud.tangle.tools',
  appIcon: '/favicon.svg',
} as const;

const walletConnectProjectId = import.meta.env.VITE_WALLETCONNECT_PROJECT_ID || '';

// Computed ONCE at module load so the same value flows into both the wagmi
// config and the autoConnect flag below. The detection reads
// `document.referrer` + `window.location` — both stable for the lifetime of
// the iframe page, so a one-shot check is safe.
// Thread `VITE_TANGLE_CLOUD_ORIGINS` (comma-separated) into the library's
// origin allowlist. The library doesn't read import.meta.env itself so it
// stays bundler-agnostic; the app injects what its env can resolve.
const EXTRA_PARENT_ORIGINS = (
  import.meta.env.VITE_TANGLE_CLOUD_ORIGINS as string | undefined
)
  ?.split(',')
  .map((s) => s.trim())
  .filter(Boolean);
const PARENT_ORIGIN = detectTangleCloudParentOrigin({
  extraOrigins: EXTRA_PARENT_ORIGINS,
});

/**
 * `true` when this app is running inside the Tangle Cloud dapp's iframe
 * shell. In that mode the wallet flows through the parent's postMessage
 * bridge instead of through a browser wallet extension (which can't inject
 * into the sandboxed iframe in the first place).
 */
export const isEmbeddedInTangleCloud = PARENT_ORIGIN !== null;

function dedupeChains(chains: readonly Chain[]): readonly [Chain, ...Chain[]] {
  const seen = new Set<number>();
  const unique = chains.filter((chain) => {
    if (seen.has(chain.id)) return false;
    seen.add(chain.id);
    return true;
  });

  return unique as [Chain, ...Chain[]];
}

function getSandboxWalletChains(): readonly [Chain, ...Chain[]] {
  return dedupeChains(Object.values(networks).map((network) => network.chain));
}

export function createWeb3Config(projectId = walletConnectProjectId): Config {
  // connectkit's `getDefaultConfig` and wagmi's `createConfig` ship slightly
  // different `Chain` / `Transport` generics across the two package versions
  // pinned by this app — they're structurally compatible at runtime. Cast
  // the shared structures explicitly via `unknown` so we don't silently
  // accept anything else.
  const walletChains = getSandboxWalletChains();
  const chains = walletChains as unknown as Parameters<typeof getDefaultConfig>[0]['chains'];
  const transports = Object.fromEntries(
    walletChains.map((chain) => [
      chain.id,
      http(networks[chain.id]?.rpcUrl ?? chain.rpcUrls.default.http[0]),
    ]),
  ) as unknown as Parameters<
    typeof getDefaultConfig
  >[0]['transports'];
  const base = getDefaultConfig({
    chains,
    transports,
    walletConnectProjectId: projectId,
    ...appMetadata,
  });
  // When embedded by Tangle Cloud, prepend the parent-bridge connector and
  // strip the rest. Browser-extension/WalletConnect/Coinbase connectors are
  // all unusable in a sandboxed iframe (no window.ethereum, no popup), and
  // surfacing them in ConnectKit's modal would only confuse operators. The
  // bridge connector auto-connects via `isAuthorized() === true`, so the
  // user never sees a wallet-picker inside the iframe; their parent-dapp
  // wallet state just flows through.
  if (PARENT_ORIGIN !== null) {
    const bridge = parentBridgeConnector({
      parentOrigin: PARENT_ORIGIN,
      appId: 'agent-sandbox',
    });
    return createConfig({
      ...base,
      connectors: [bridge],
    });
  }
  return createConfig(base);
}

const config = createWeb3Config();

export { config };

export function Web3Provider({ children }: { children: ReactNode }) {
  return (
    // Web3Shell owns the reconnect workaround for this stack. Keeping app-level
    // wallet restore logic out of this provider avoids duplicating connector
    // heuristics and prevents route-specific reconnect hacks here.
    <Web3Shell config={config}>
      <ConnectKitProvider
        theme="auto"
        mode="auto"
        options={{
          ...defaultConnectKitOptions,
          initialChainId: baseSepolia.id,
        }}
      >
        {children}
      </ConnectKitProvider>
    </Web3Shell>
  );
}
