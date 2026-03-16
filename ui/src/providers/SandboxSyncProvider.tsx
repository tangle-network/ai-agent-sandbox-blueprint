import { type ReactNode, useEffect, useRef } from 'react';
import { useAccount } from 'wagmi';
import { useSandboxHydration } from '~/lib/hooks/useSandboxHydration';

const POLL_INTERVAL_MS = 5_000;

function makeAuthAttemptKey(address: string) {
  return address.toLowerCase();
}

export function SandboxSyncProvider({ children }: { children: ReactNode }) {
  const { address, isConnected } = useAccount();
  const { refresh, authRequired, isHydrating } = useSandboxHydration();
  const autoAuthAttemptKeyRef = useRef<string | null>(null);

  useEffect(() => {
    if (!isConnected || !address) {
      autoAuthAttemptKeyRef.current = null;
      return;
    }

    const nextKey = makeAuthAttemptKey(address);
    if (autoAuthAttemptKeyRef.current && autoAuthAttemptKeyRef.current !== nextKey) {
      autoAuthAttemptKeyRef.current = null;
    }
  }, [address, isConnected]);

  useEffect(() => {
    if (!isConnected || !address || !authRequired || isHydrating) return;
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;

    const attemptKey = makeAuthAttemptKey(address);
    if (autoAuthAttemptKeyRef.current === attemptKey) return;

    autoAuthAttemptKeyRef.current = attemptKey;
    void refresh({ interactive: true });
  }, [address, authRequired, isConnected, isHydrating, refresh]);

  useEffect(() => {
    if (!isConnected || !address) return;
    if (typeof window === 'undefined') return;

    const intervalId = window.setInterval(() => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      void refresh({ interactive: false });
    }, POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [address, isConnected, refresh]);

  return <>{children}</>;
}
