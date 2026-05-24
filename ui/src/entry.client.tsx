// Must be the FIRST import — polyfills crypto.randomUUID before any wallet
// library (wagmi, ConnectKit, WalletConnect) evaluates. Without this, those
// libraries fail silently on insecure contexts (HTTP over LAN/Tailscale),
// causing 10-15s reconnection delays or broken wallet persistence.
import './polyfills';

import { startTransition } from 'react';
import { hydrateRoot } from 'react-dom/client';
import { HydratedRouter } from 'react-router/dom';

// Migrate localStorage keys from sandbox_cloud_* → bp_* (one-time).
// Wrapped in `withLocalStorage` so this no-ops cleanly when the UI is loaded
// inside the Tangle Cloud dapp's sandboxed iframe (no `allow-same-origin`,
// so any `localStorage` access throws SecurityError — an unhandled throw
// here used to prevent `hydrateRoot` from ever running and produced a blank
// black void inside the dapp's iframe shell).
import { withLocalStorage } from '~/lib/safe-storage';
const KEY_MIGRATIONS: [string, string][] = [
  ['sandbox_cloud_theme', 'bp_theme'],
  ['sandbox_cloud_tx_history', 'bp_tx_history'],
  ['sandbox_cloud_sessions', 'bp_sessions'],
  ['sandbox_cloud_infra', 'bp_infra'],
  ['sandbox_cloud_selected_chain', 'bp_selected_chain'],
];
withLocalStorage((ls) => {
  for (const [oldKey, newKey] of KEY_MIGRATIONS) {
    if (!ls.getItem(newKey) && ls.getItem(oldKey)) {
      ls.setItem(newKey, ls.getItem(oldKey)!);
    }
  }
});

// Ensure chains module (with configureNetworks) is loaded early
import('~/lib/contracts/chains');

startTransition(() => {
  hydrateRoot(document, <HydratedRouter />);
});
