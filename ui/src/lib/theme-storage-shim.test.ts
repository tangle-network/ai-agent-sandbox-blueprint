import { describe, expect, it } from 'vitest';
import { buildInlineThemeBootstrap } from './theme';

/**
 * The inline bootstrap installs an in-memory `Storage` shim when the real
 * `window.localStorage` throws on access (sandboxed iframe with no
 * `allow-same-origin`). It must run before any module evaluates so that
 * downstream wagmi / ConnectKit / blueprint-ui storage reads succeed.
 *
 * Test strategy: build the script source, evaluate it against a fabricated
 * `window` whose `localStorage` getter throws — same shape as the
 * SecurityError the browser produces in a sandboxed iframe — and assert
 * the script replaces `window.localStorage` with a working shim.
 */
describe('buildInlineThemeBootstrap → localStorage shim', () => {
  function evalAgainstWindow(fakeWindow: object): object {
    const fakeDoc = {
      documentElement: {
        setAttribute: () => {
          /* no-op for shim test */
        },
        querySelector: () => null,
      },
      querySelector: () => fakeDoc.documentElement,
    };
    class FakeURL {
      searchParams: URLSearchParams;
      constructor(href: string) {
        const idx = href.indexOf('?');
        this.searchParams = new URLSearchParams(idx >= 0 ? href.slice(idx) : '');
      }
    }
    const src = buildInlineThemeBootstrap();
    // eslint-disable-next-line @typescript-eslint/no-implied-eval
    const fn = new Function('window', 'document', 'URL', src);
    fn(fakeWindow, fakeDoc, FakeURL);
    return fakeWindow;
  }

  it('replaces window.localStorage with an in-memory shim when access throws', () => {
    // SecurityError-style: the GET on `localStorage` itself throws (mirrors
    // sandboxed-iframe DOMException). The original property must be
    // configurable so the inline script can redefine it as a data property.
    const fakeWindow: Record<string, unknown> = {
      location: { href: 'https://x.example/?theme=dark' },
      matchMedia: () => ({ matches: true }),
    };
    Object.defineProperty(fakeWindow, 'localStorage', {
      configurable: true,
      get() {
        throw new DOMException(
          "Failed to read the 'localStorage' property from 'Window'",
          'SecurityError',
        );
      },
    });
    Object.defineProperty(fakeWindow, 'sessionStorage', {
      configurable: true,
      get() {
        throw new DOMException('blocked', 'SecurityError');
      },
    });
    const after = evalAgainstWindow(fakeWindow) as {
      localStorage: Storage;
      sessionStorage: Storage;
    };
    // After the script runs, both Storage references must be the in-memory
    // shim (a real object, not a thrown access).
    expect(() => after.localStorage).not.toThrow();
    expect(() => after.sessionStorage).not.toThrow();
    expect(typeof after.localStorage.getItem).toBe('function');
    expect(typeof after.localStorage.setItem).toBe('function');
    expect(typeof after.localStorage.removeItem).toBe('function');
    // The shim must actually persist values for the lifetime of the document.
    after.localStorage.setItem('hello', 'world');
    expect(after.localStorage.getItem('hello')).toBe('world');
    after.localStorage.removeItem('hello');
    expect(after.localStorage.getItem('hello')).toBeNull();
    // `length` must reflect the in-memory state so libraries that iterate
    // storage (some wallet plugins) see a coherent view.
    after.localStorage.setItem('a', '1');
    after.localStorage.setItem('b', '2');
    expect(after.localStorage.length).toBe(2);
  });

  it('leaves a working localStorage untouched', () => {
    const realMap = new Map<string, string>();
    const realStorage: Storage = {
      get length() {
        return realMap.size;
      },
      clear: () => realMap.clear(),
      getItem: (k) => realMap.get(k) ?? null,
      key: (i) => Array.from(realMap.keys())[i] ?? null,
      removeItem: (k) => {
        realMap.delete(k);
      },
      setItem: (k, v) => {
        realMap.set(k, v);
      },
    };
    const fakeWindow = {
      localStorage: realStorage,
      sessionStorage: realStorage,
      location: { href: 'https://x.example/?theme=light' },
      matchMedia: () => ({ matches: false }),
    };
    const after = evalAgainstWindow(fakeWindow) as { localStorage: Storage };
    // The script must NOT have replaced the working Storage — same identity.
    // (Whether the URL-theme write lands on `window.localStorage` vs the
    // global `localStorage` is a harness detail — the contract this test
    // protects is "do not clobber a working Storage with a shim".)
    expect(after.localStorage).toBe(realStorage);
  });
});
