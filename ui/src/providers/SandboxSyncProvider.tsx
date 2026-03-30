import { type ReactNode, useEffect, useRef } from 'react';
import { useAccount } from 'wagmi';
import { useSandboxHydration } from '~/lib/hooks/useSandboxHydration';
import { useInstanceHydration } from '~/lib/hooks/useInstanceHydration';

const POLL_INTERVAL_MS = 5_000;

function makeAuthAttemptKey(address: string) {
  return address.toLowerCase();
}

export function SandboxSyncProvider({ children }: { children: ReactNode }) {
  const { address, isConnected } = useAccount();
  const {
    refresh: refreshSandboxes,
    authRequired: sandboxAuthRequired,
    isHydrating: sandboxesHydrating,
  } = useSandboxHydration();
  const {
    refresh: refreshInstances,
    authRequired: instanceAuthRequired,
    isHydrating: instancesHydrating,
  } = useInstanceHydration();
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
    const needsAuth = sandboxAuthRequired || instanceAuthRequired;
    const isBusy = sandboxesHydrating || instancesHydrating;
    if (!isConnected || !address || !needsAuth || isBusy) return;
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;

    const attemptKey = makeAuthAttemptKey(address);
    if (autoAuthAttemptKeyRef.current === attemptKey) return;

    autoAuthAttemptKeyRef.current = attemptKey;
    void Promise.all([
      refreshSandboxes({ interactive: true }),
      refreshInstances({ interactive: true }),
    ]);
  }, [
    address,
    isConnected,
    instanceAuthRequired,
    instancesHydrating,
    refreshInstances,
    refreshSandboxes,
    sandboxAuthRequired,
    sandboxesHydrating,
  ]);

  useEffect(() => {
    if (!isConnected || !address) return;
    if (typeof window === 'undefined') return;

    const intervalId = window.setInterval(() => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      void Promise.all([
        refreshSandboxes({ interactive: false }),
        refreshInstances({ interactive: false }),
      ]);
    }, POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [address, isConnected, refreshInstances, refreshSandboxes]);

  return <>{children}</>;
}
