import { startTransition } from 'react';
import { hydrateRoot } from 'react-dom/client';
import { HydratedRouter } from 'react-router/dom';

// Migrate localStorage keys from sandbox_cloud_* → bp_* (one-time)
const KEY_MIGRATIONS: [string, string][] = [
  ['sandbox_cloud_theme', 'bp_theme'],
  ['sandbox_cloud_tx_history', 'bp_tx_history'],
  ['sandbox_cloud_sessions', 'bp_sessions'],
  ['sandbox_cloud_infra', 'bp_infra'],
  ['sandbox_cloud_selected_chain', 'bp_selected_chain'],
];
for (const [oldKey, newKey] of KEY_MIGRATIONS) {
  if (!localStorage.getItem(newKey) && localStorage.getItem(oldKey)) {
    localStorage.setItem(newKey, localStorage.getItem(oldKey)!);
  }
}

// Ensure chains module (with configureNetworks) is loaded early
import('~/lib/contracts/chains');

startTransition(() => {
  hydrateRoot(document, <HydratedRouter />);
});
