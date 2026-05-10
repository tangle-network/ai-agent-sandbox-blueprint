import { createConfig, type Config } from 'wagmi';
import { ConnectKitProvider, getDefaultConfig } from 'connectkit';
import { type ReactNode } from 'react';
import {
  createTangleTransports,
  defaultConnectKitOptions,
  tangleWalletChains,
} from '@tangle-network/blueprint-ui';
import { Web3Shell } from '@tangle-network/blueprint-ui/components';

const appMetadata = {
  appName: 'Tangle Sandbox Cloud',
  appDescription: 'AI Agent Sandbox Provisioning on Tangle Network',
  appUrl: typeof window !== 'undefined' ? window.location.origin : 'https://cloud.tangle.tools',
  appIcon: '/favicon.svg',
} as const;

const walletConnectProjectId = import.meta.env.VITE_WALLETCONNECT_PROJECT_ID || '';

export function createWeb3Config(projectId = walletConnectProjectId): Config {
  // connectkit's `getDefaultConfig` and wagmi's `createConfig` ship slightly
  // different `Chain` / `Transport` generics across the two package versions
  // pinned by this app — they're structurally compatible at runtime. Cast
  // the shared structures explicitly via `unknown` so we don't silently
  // accept anything else.
  const chains = tangleWalletChains as unknown as Parameters<typeof getDefaultConfig>[0]['chains'];
  const transports = createTangleTransports() as unknown as Parameters<
    typeof getDefaultConfig
  >[0]['transports'];
  return createConfig(
    getDefaultConfig({
      chains,
      transports,
      walletConnectProjectId: projectId,
      ...appMetadata,
    }),
  );
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
          initialChainId: undefined,
        }}
      >
        {children}
      </ConnectKitProvider>
    </Web3Shell>
  );
}
