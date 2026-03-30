import { createConfig } from 'wagmi';
import { ConnectKitProvider, getDefaultConfig } from 'connectkit';
import { type ReactNode } from 'react';
import {
  createTangleTransports,
  defaultConnectKitOptions,
  getTangleWalletChains,
} from '@tangle-network/blueprint-ui';
import { Web3Shell } from '@tangle-network/blueprint-ui/components';
import { tangleLocal } from '~/lib/contracts/chains';

const appMetadata = {
  appName: 'Tangle Sandbox Cloud',
  appDescription: 'AI Agent Sandbox Provisioning on Tangle Network',
  appUrl: typeof window !== 'undefined' ? window.location.origin : 'https://cloud.tangle.tools',
  appIcon: '/favicon.svg',
} as const;

const walletConnectProjectId = import.meta.env.VITE_WALLETCONNECT_PROJECT_ID || '';

export function createWeb3Config(projectId = walletConnectProjectId) {
  return createConfig(
    getDefaultConfig({
      chains: getTangleWalletChains(tangleLocal) as any,
      transports: createTangleTransports(tangleLocal) as any,
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
    <Web3Shell config={config as any}>
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
