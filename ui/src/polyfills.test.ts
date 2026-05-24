import { describe, expect, it, beforeEach, afterEach } from 'vitest';

describe('storage shim (sandboxed iframe)', () => {
  let originalLocal: PropertyDescriptor | undefined;
  let originalSession: PropertyDescriptor | undefined;
  beforeEach(() => {
    originalLocal = Object.getOwnPropertyDescriptor(window, 'localStorage');
    originalSession = Object.getOwnPropertyDescriptor(window, 'sessionStorage');
  });
  afterEach(() => {
    if (originalLocal) Object.defineProperty(window, 'localStorage', originalLocal);
    if (originalSession) Object.defineProperty(window, 'sessionStorage', originalSession);
    // Bust the module cache so the polyfill re-runs in the next test.
    vitestResetModules();
  });

  function vitestResetModules() {
    // dynamic import re-evaluates the side-effectful module
  }

  it('installs an in-memory shim when localStorage property access throws', async () => {
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() { throw new DOMException('access denied', 'SecurityError'); },
    });
    await import('./polyfills' + '?case=ls-throws');
    // Now localStorage should be a writable in-memory shim.
    expect(() => window.localStorage.setItem('k', 'v')).not.toThrow();
    expect(window.localStorage.getItem('k')).toBe('v');
    expect(window.localStorage.length).toBe(1);
    window.localStorage.removeItem('k');
    expect(window.localStorage.getItem('k')).toBeNull();
  });

  it('leaves a working localStorage alone', async () => {
    // jsdom provides a working Storage by default. The polyfill must not
    // clobber it.
    const original = window.localStorage;
    await import('./polyfills' + '?case=ls-works');
    expect(window.localStorage).toBe(original);
  });

  it('installs a sessionStorage shim independently', async () => {
    Object.defineProperty(window, 'sessionStorage', {
      configurable: true,
      get() { throw new DOMException('access denied', 'SecurityError'); },
    });
    await import('./polyfills' + '?case=ss-throws');
    expect(() => window.sessionStorage.setItem('x', '1')).not.toThrow();
    expect(window.sessionStorage.getItem('x')).toBe('1');
  });
});
