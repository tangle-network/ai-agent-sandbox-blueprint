import { useEffect, useState } from 'react';

const WEI_PER_ETH = 1_000_000_000_000_000_000n;
const DISPLAY_SCALE = 1_000n; // 3 decimal places

function formatEthBalance(wei: bigint): string {
  const whole = wei / WEI_PER_ETH;
  const fraction = ((wei % WEI_PER_ETH) * DISPLAY_SCALE) / WEI_PER_ETH;
  return `${whole.toString()}.${fraction.toString().padStart(3, '0')}`;
}

interface UseWalletEthBalanceOptions {
  address?: string;
  refreshKey?: number | string;
  readBalance: (address: string) => Promise<bigint>;
  pollMs?: number;
  onError?: (error: unknown) => void;
}

interface UseWalletEthBalanceResult {
  balance: string | null;
  hasError: boolean;
}

export function useWalletEthBalance({
  address,
  refreshKey,
  readBalance,
  pollMs = 15_000,
  onError,
}: UseWalletEthBalanceOptions): UseWalletEthBalanceResult {
  const [balance, setBalance] = useState<string | null>(null);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    if (!address) {
      setBalance(null);
      setHasError(false);
      return;
    }

    let cancelled = false;

    const fetchBalance = () => {
      readBalance(address)
        .then((wei) => {
          if (cancelled) return;
          setBalance(formatEthBalance(wei));
          setHasError(false);
        })
        .catch((error: unknown) => {
          if (cancelled) return;
          setHasError(true);
          onError?.(error);
        });
    };

    fetchBalance();
    const interval = setInterval(fetchBalance, pollMs);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [address, refreshKey, readBalance, pollMs, onError]);

  return { balance, hasError };
}
