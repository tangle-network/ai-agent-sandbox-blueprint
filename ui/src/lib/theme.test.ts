import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { THEME_STORAGE_KEYS, buildInlineThemeBootstrap } from './theme';

// The inline theme bootstrap is shipped as a string and evaluated by the
// browser before any React code runs. We can't import it as JS, so the test
// strategy is: build the source, wrap it in a function with stubbed globals
// (window, localStorage, document, matchMedia), then assert the side-effects.

interface BootstrapHarness {
  href: string;
  store: Map<string, string>;
  prefersDark: boolean;
}

function runBootstrap(harness: BootstrapHarness): string | null {
  const setAttr = { value: null as string | null };
  const fakeStorage = {
    getItem: (key: string) => harness.store.get(key) ?? null,
    setItem: (key: string, value: string) => {
      harness.store.set(key, value);
    },
  };
  const fakeDoc = {
    documentElement: {
      setAttribute: (_name: string, value: string) => {
        setAttr.value = value;
      },
    },
  };
  const fakeWindow = {
    location: { href: harness.href },
    matchMedia: (query: string) => ({
      matches: query.includes('dark') ? harness.prefersDark : false,
    }),
  };
  const fakeURL = class {
    searchParams: URLSearchParams;
    constructor(href: string) {
      const idx = href.indexOf('?');
      this.searchParams = new URLSearchParams(idx >= 0 ? href.slice(idx) : '');
    }
  };
  const src = buildInlineThemeBootstrap();
  // eslint-disable-next-line @typescript-eslint/no-implied-eval
  const fn = new Function(
    'window',
    'document',
    'localStorage',
    'URL',
    src,
  );
  fn(fakeWindow, fakeDoc, fakeStorage, fakeURL);
  return setAttr.value;
}

describe('buildInlineThemeBootstrap', () => {
  let store: Map<string, string>;

  beforeEach(() => {
    store = new Map();
  });

  afterEach(() => {
    store.clear();
  });

  it('honors ?theme=light from URL on first paint', () => {
    const theme = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/?theme=light',
      store,
      prefersDark: true,
    });
    expect(theme).toBe('light');
    // URL should clobber localStorage so themeStore picks it up
    expect(store.get(THEME_STORAGE_KEYS[0])).toBe('light');
  });

  it('honors ?theme=dark from URL on first paint', () => {
    const theme = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/?mode=default&blueprintId=12&theme=dark',
      store,
      prefersDark: false,
    });
    expect(theme).toBe('dark');
    expect(store.get(THEME_STORAGE_KEYS[0])).toBe('dark');
  });

  it('URL theme overrides a persisted user preference', () => {
    // User previously toggled to dark; iframe is reloaded by a light-themed parent.
    store.set(THEME_STORAGE_KEYS[0], 'dark');
    const theme = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/?theme=light',
      store,
      prefersDark: true,
    });
    expect(theme).toBe('light');
    expect(store.get(THEME_STORAGE_KEYS[0])).toBe('light');
  });

  it('ignores invalid ?theme values', () => {
    const theme = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/?theme=neon',
      store,
      prefersDark: false,
    });
    // Invalid → fall back to localStorage (empty) → system pref → light
    expect(theme).toBe('light');
  });

  it('falls back to persisted theme when URL has no param', () => {
    store.set(THEME_STORAGE_KEYS[0], 'light');
    const theme = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/',
      store,
      prefersDark: true,
    });
    expect(theme).toBe('light');
  });

  it('migrates from legacy storage key when primary is unset', () => {
    store.set(THEME_STORAGE_KEYS[1], 'light');
    const theme = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/',
      store,
      prefersDark: true,
    });
    expect(theme).toBe('light');
  });

  it('falls back to system preference when nothing is set', () => {
    const dark = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/',
      store,
      prefersDark: true,
    });
    expect(dark).toBe('dark');

    store.clear();
    const light = runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/',
      store,
      prefersDark: false,
    });
    expect(light).toBe('light');
  });

  it('does not write to localStorage when URL has no theme', () => {
    // System pref should drive first paint, but must NOT persist — otherwise
    // a user who never picked a theme would have their preference frozen.
    runBootstrap({
      href: 'https://agent-sandbox.blueprint.tangle.tools/',
      store,
      prefersDark: true,
    });
    expect(store.has(THEME_STORAGE_KEYS[0])).toBe(false);
    expect(store.has(THEME_STORAGE_KEYS[1])).toBe(false);
  });
});
