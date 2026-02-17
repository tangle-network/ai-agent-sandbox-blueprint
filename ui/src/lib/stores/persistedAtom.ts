import { atom, type WritableAtom } from 'nanostores';

export function serializeWithBigInt(value: unknown): string {
  return JSON.stringify(value, (_key, v) =>
    typeof v === 'bigint' ? { __bigint: v.toString() } : v,
  );
}

export function deserializeWithBigInt<T>(raw: string): T {
  return JSON.parse(raw, (_key, v) => {
    if (v && typeof v === 'object' && '__bigint' in v && typeof v.__bigint === 'string') {
      return BigInt(v.__bigint);
    }
    return v;
  }) as T;
}

interface PersistedAtomOpts<T> {
  key: string;
  initial: T;
  serialize?: (value: T) => string;
  deserialize?: (raw: string) => T;
}

export function persistedAtom<T>(opts: PersistedAtomOpts<T>): WritableAtom<T> {
  const { key, initial, serialize = JSON.stringify, deserialize = JSON.parse } = opts;

  let restored = initial;
  if (typeof window !== 'undefined') {
    try {
      const raw = localStorage.getItem(key);
      if (raw !== null) {
        restored = deserialize(raw);
      }
    } catch {
      // corrupt data
    }
  }

  const store = atom<T>(restored);

  if (typeof window !== 'undefined') {
    store.subscribe((value) => {
      try {
        localStorage.setItem(key, serialize(value));
      } catch {
        // storage full
      }
    });
  }

  return store;
}
