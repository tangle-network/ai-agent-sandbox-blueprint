import { useSignMessage } from 'wagmi';
import { useSidecarAuth } from './useSidecarAuth';

/**
 * Adapter hook wiring wagmi wallet signing to generic sidecar auth.
 */
export function useWagmiSidecarAuth(resourceId: string, apiUrl: string) {
  const { signMessageAsync } = useSignMessage();
  return useSidecarAuth({
    resourceId,
    apiUrl,
    signMessage: (msg: string) => signMessageAsync({ message: msg }),
  });
}
