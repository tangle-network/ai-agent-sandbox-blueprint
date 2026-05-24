// Iframe-safe localStorage access.
//
// When this UI is embedded by the Tangle Cloud dapp, the parent iframe sandbox
// is `allow-scripts allow-forms` — no `allow-same-origin`. In that mode the
// document is forced to an opaque origin and *any* access to
// `window.localStorage` throws a SecurityError. Even the property read
// `window.localStorage` itself throws — `typeof window !== 'undefined' &&
// window.localStorage` doesn't short-circuit safely.
//
// That uncaught error during module evaluation prevented `hydrateRoot` from
// ever running, so the embedded sandbox UI rendered as a blank black void
// inside the dapp (while the standalone domain worked fine because top-frame
// localStorage is accessible).
//
// This helper centralizes the try/catch. Callers should prefer
// `withLocalStorage(s => ...)` over directly accessing `window.localStorage`,
// and treat the embedded-iframe case (no storage) as "no persisted state".

/**
 * Return the document's `Storage` if it is accessible, otherwise `null`.
 *
 * Accessing `window.localStorage` synchronously throws in sandboxed iframes
 * without `allow-same-origin`, so this read MUST be inside a try/catch.
 * `typeof` and `&&` guards do not help — the property access itself is the
 * point that throws.
 */
export function getSafeLocalStorage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/**
 * Run `fn` with the document's `Storage` if accessible. Swallows storage
 * errors (quota exceeded, security policy, sandboxed iframe). The callback's
 * return value is propagated; when storage is unusable the function returns
 * `undefined` and `fn` is not called.
 *
 * Use this for one-off reads/writes; for module-init guards prefer
 * `getSafeLocalStorage()` so the call site stays readable.
 */
export function withLocalStorage<T>(fn: (storage: Storage) => T): T | undefined {
  const storage = getSafeLocalStorage();
  if (!storage) return undefined;
  try {
    return fn(storage);
  } catch {
    return undefined;
  }
}
