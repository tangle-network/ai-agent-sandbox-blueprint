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
 * evaluates. Setting `data-theme` here avoids a flash of the wrong theme and
 * lets `themeStore` from blueprint-ui pick up the URL-derived value at
 * initialization time.
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
        var url = new URL(window.location.href);
        var fromUrl = url.searchParams.get(${PARAM});
        var theme = null;
        if (fromUrl === 'light' || fromUrl === 'dark') {
          theme = fromUrl;
          // Parent dictates theme on every load — keep persisted value in sync
          // so themeStore (which reads localStorage first) honors the URL too.
          try { localStorage.setItem(${PRIMARY}, theme); } catch (e) {}
        } else {
          theme = localStorage.getItem(${PRIMARY}) || localStorage.getItem(${SECONDARY});
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
