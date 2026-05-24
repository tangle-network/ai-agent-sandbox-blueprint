// Theme resolution for the iframe app.
//
// The iframe is embedded by the Tangle Cloud dapp shell, which publishes its
// active theme through a reserved URL contract:
//
//   https://agent-sandbox.blueprint.tangle.tools/?theme=light|dark
//
// (see apps/tangle-cloud/src/blueprintApps/iframe/README.md in the dapp).
//
// The parent's theme is set FRESH on every iframe load, so the URL param wins
// over any persisted user preference at first paint. Once the user toggles
// theme inside the iframe, `toggleTheme()` from @tangle-network/blueprint-ui
// writes to `bp_theme` localStorage; that toggle sticks for the current
// session but is overridden on the next parent-issued reload.
//
// When the iframe is loaded standalone (no `?theme=` in URL), behavior falls
// back to the existing localStorage + system-pref chain so the standalone
// domain works the same as before.

export const THEME_STORAGE_KEYS = ['bp_theme', 'sandbox_cloud_theme'] as const;
export const THEME_URL_PARAM = 'theme';

export type Theme = 'dark' | 'light';

/**
 * Inline-script source injected into the document head before any React code
 * evaluates. Two responsibilities:
 *
 *  1. Set `data-theme` so first paint matches the parent shell's theme.
 *  2. Install an in-memory `localStorage` / `sessionStorage` shim when the
 *     real storage is inaccessible (sandboxed iframe with no
 *     `allow-same-origin`).
 *
 * The shim MUST be installed here — in an inline script that runs during HTML
 * parse — not in a module. Modules (including `polyfills.ts`) execute AFTER
 * HTML parse finishes, but the inline theme script and any third-party
 * `<link rel="modulepreload">`-resolved modules can race the first
 * `localStorage` access. wagmi, ConnectKit, viem, and parts of
 * `@tangle-network/blueprint-ui` all touch storage at module-init, so the
 * shim has to be live before any of them evaluate. The inline script is the
 * only point in the lifecycle that guarantees this ordering.
 *
 * Trading Arena uses the identical strategy — they hit the same iframe-mount
 * regression and this is the working fix. We deliberately mirror their
 * shape so a future bundler/version change can't introduce drift.
 *
 * The script must be self-contained (no imports) and side-effect-only.
 */
export function buildInlineThemeBootstrap(): string {
  const [primaryKey, secondaryKey] = THEME_STORAGE_KEYS;
  // Stringify constants so they survive the inline boundary unchanged.
  const PRIMARY = JSON.stringify(primaryKey);
  const SECONDARY = JSON.stringify(secondaryKey);
  const PARAM = JSON.stringify(THEME_URL_PARAM);
  return `
    (function () {
      try {
        // localStorage / sessionStorage shim — only installs when the real
        // Storage is inaccessible. Probe BOTH get and set because some
        // environments throw only on mutation (private-mode Safari
        // historically; Firefox 'block cookies'), but the sandboxed-iframe
        // case typically throws on either.
        var needsShim = false;
        try {
          window.localStorage.getItem('__bp_probe__');
          window.localStorage.setItem('__bp_probe__', '1');
          window.localStorage.removeItem('__bp_probe__');
        } catch (_probeErr) {
          needsShim = true;
        }
        if (needsShim) {
          var memory = {};
          var shim = {
            getItem: function (k) { return Object.prototype.hasOwnProperty.call(memory, k) ? memory[k] : null; },
            setItem: function (k, v) { memory[k] = String(v); },
            removeItem: function (k) { delete memory[k]; },
            clear: function () { memory = {}; },
            key: function (i) { return Object.keys(memory)[i] || null; },
          };
          Object.defineProperty(shim, 'length', { get: function () { return Object.keys(memory).length; } });
          // Use a data descriptor (value:) rather than an accessor (get:):
          // in some sandboxed-iframe contexts the WebIDL [LegacyUnforgeable]
          // localStorage getter is not fully overridden by an accessor
          // redefinition, but a data property replacement works. Wrap in
          // try/catch because some environments lock the descriptor.
          try {
            Object.defineProperty(window, 'localStorage', { value: shim, configurable: true });
            Object.defineProperty(window, 'sessionStorage', { value: shim, configurable: true });
          } catch (_defErr) {}
        }

        var url = new URL(window.location.href);
        var fromUrl = url.searchParams.get(${PARAM});
        var theme = null;
        if (fromUrl === 'light' || fromUrl === 'dark') {
          theme = fromUrl;
          // Parent dictates theme on every load — keep persisted value in sync
          // so themeStore (which reads localStorage first) honors the URL too.
          try { localStorage.setItem(${PRIMARY}, theme); } catch (e) {}
        } else {
          try {
            theme = localStorage.getItem(${PRIMARY}) || localStorage.getItem(${SECONDARY});
          } catch (_getErr) {}
        }
        if (theme !== 'light' && theme !== 'dark') {
          theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
        }
        document.documentElement.setAttribute('data-theme', theme);
      } catch (e) {
        document.documentElement.setAttribute('data-theme', 'dark');
      }
    })();
  `;
}
