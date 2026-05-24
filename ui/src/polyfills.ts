/**
 * Polyfill crypto.randomUUID for insecure contexts (HTTP over LAN/Tailscale).
 *
 * Browsers restrict crypto.randomUUID() to secure contexts (HTTPS/localhost),
 * but wallet extensions (ConnectKit, WalletConnect) and wagmi depend on it.
 *
 * IMPORTANT: This module must be the first import in root.tsx so it runs
 * before any wallet/web3 module evaluation. ESM imports are hoisted, so
 * inline polyfill code in root.tsx would run AFTER its imports are evaluated.
 */
if (typeof crypto !== 'undefined' && !crypto.randomUUID) {
  crypto.randomUUID = () =>
    // @ts-expect-error — crypto.getRandomValues is available in all contexts
    ([1e7] + -1e3 + -4e3 + -8e3 + -1e11).replace(/[018]/g, (c: number) =>
      (c ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (c / 4)))).toString(16),
    ) as `${string}-${string}-${string}-${string}-${string}`;
}

/**
 * Install an in-memory `localStorage` / `sessionStorage` shim when the real
 * Storage is inaccessible (sandboxed iframe with no `allow-same-origin`).
 *
 * The Tangle Cloud dapp embeds this UI with an iframe `sandbox="allow-scripts
 * allow-forms"` policy. That forces an opaque origin, and **every** access
 * to `window.localStorage` — including the property read — throws
 * SecurityError. We cannot fix every consumer (wagmi, ConnectKit, viem,
 * blueprint-ui internals all touch localStorage at module-init time and the
 * library code is not ours to wrap), so we replace the throwing property
 * with a writable in-memory shim before any other module evaluates.
 *
 * Wagmi/ConnectKit happily persist wallet state into the shim — it just
 * doesn't survive a page reload, which is the right behavior for an
 * embedded sub-app anyway (the parent dapp owns the wallet session).
 *
 * Must run BEFORE any wallet/web3 module imports. ESM hoists imports, so
 * this code lives in `polyfills.ts` (the very first import in
 * entry.client.tsx) rather than inline at a use site.
 */
function installStorageShim(target: 'localStorage' | 'sessionStorage'): void {
  if (typeof window === 'undefined') return;
  // Probe — if the real Storage is accessible, leave it alone.
  try {
    const probe = window[target];
    if (probe) {
      probe.getItem('__storage_probe__');
      return;
    }
  } catch {
    // Property access itself threw — fall through to install the shim.
  }

  const memory = new Map<string, string>();
  const shim: Storage = {
    get length() {
      return memory.size;
    },
    clear() {
      memory.clear();
    },
    getItem(key: string) {
      return memory.has(key) ? (memory.get(key) as string) : null;
    },
    key(index: number) {
      return Array.from(memory.keys())[index] ?? null;
    },
    removeItem(key: string) {
      memory.delete(key);
    },
    setItem(key: string, value: string) {
      memory.set(key, String(value));
    },
  };

  try {
    Object.defineProperty(window, target, {
      configurable: true,
      get() {
        return shim;
      },
    });
  } catch {
    // Defining the property failed too (extremely locked-down env). At this
    // point we can't help — any subsequent module-init localStorage access
    // will still throw — but the failure path is identical to today, so we
    // are no worse off than before.
  }
}

installStorageShim('localStorage');
installStorageShim('sessionStorage');
