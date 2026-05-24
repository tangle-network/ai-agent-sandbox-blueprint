import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getSafeLocalStorage, withLocalStorage } from './safe-storage';

describe('safe-storage', () => {
  // Snapshot the original descriptor so we can restore it between tests —
  // jsdom installs a real Storage by default and we replace it per-case.
  let originalDescriptor: PropertyDescriptor | undefined;
  beforeEach(() => {
    originalDescriptor = Object.getOwnPropertyDescriptor(window, 'localStorage');
  });
  afterEach(() => {
    if (originalDescriptor) Object.defineProperty(window, 'localStorage', originalDescriptor);
  });

  it('returns the Storage when accessible', () => {
    expect(getSafeLocalStorage()).toBe(window.localStorage);
  });

  it('returns null when property access itself throws (sandboxed iframe)', () => {
    // Simulate the sandboxed-iframe SecurityError: reading window.localStorage
    // throws synchronously rather than returning a Storage.
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() { throw new DOMException('access denied', 'SecurityError'); },
    });
    expect(getSafeLocalStorage()).toBeNull();
  });

  it('withLocalStorage swallows setItem failures', () => {
    // Storage exists but writes throw (quota exceeded, etc.) — withLocalStorage
    // must return undefined and not propagate.
    const failingStorage = {
      getItem: () => null,
      setItem: () => { throw new DOMException('quota exceeded', 'QuotaExceededError'); },
      removeItem: vi.fn(),
      clear: vi.fn(),
      key: () => null,
      length: 0,
    } as Storage;
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() { return failingStorage; },
    });
    expect(withLocalStorage((ls) => ls.setItem('k', 'v'))).toBeUndefined();
  });

  it('withLocalStorage does not invoke the callback when storage is missing', () => {
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() { throw new DOMException('access denied', 'SecurityError'); },
    });
    const fn = vi.fn();
    expect(withLocalStorage(fn)).toBeUndefined();
    expect(fn).not.toHaveBeenCalled();
  });
});
