import { useSignMessage } from 'wagmi';
import { useSidecarAuth } from './useSidecarAuth';

/**
 * Adapter hook wiring wagmi wallet signing to generic sidecar auth.
 *
 * @deprecated Browser applications should prefer operator-proxied access
 * instead of direct sidecar authentication.
 */
export function useWagmiSidecarAuth(resourceId: string, apiUrl: string) {
  const { signMessageAsync } = useSignMessage();
  return useSidecarAuth({
    resourceId,
    apiUrl,
    signMessage: (msg: string) => signMessageAsync({ message: msg }),
  });
}
