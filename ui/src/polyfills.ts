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
