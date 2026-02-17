import { useState, useEffect } from 'react';
import type { Address } from 'viem';
import { tangleOperatorsAbi } from '~/lib/contracts/abi';
import { publicClient, getAddresses } from '~/lib/contracts/publicClient';

export interface DiscoveredOperator {
  address: Address;
  ecdsaPublicKey: string;
  rpcAddress: string;
}

/**
 * Discover operators registered for a blueprint by scanning OperatorRegistered
 * events, then verifying each is still active via multicall.
 */
export function useOperators(blueprintId: bigint) {
  const [operators, setOperators] = useState<DiscoveredOperator[]>([]);
  const [operatorCount, setOperatorCount] = useState<bigint>(0n);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    let cancelled = false;
    const addrs = getAddresses();

    async function discover() {
      setIsLoading(true);
      setError(null);

      try {
        // Step 1: Check operator count
        const count = await publicClient.readContract({
          address: addrs.services,
          abi: tangleOperatorsAbi,
          functionName: 'blueprintOperatorCount',
          args: [blueprintId],
        });
        if (cancelled) return;
        setOperatorCount(count as bigint);

        if ((count as bigint) === 0n) {
          setOperators([]);
          setIsLoading(false);
          return;
        }

        // Step 2: Fetch OperatorRegistered logs
        const registeredLogs = await publicClient.getLogs({
          address: addrs.services,
          event: {
            type: 'event' as const,
            name: 'OperatorRegistered',
            inputs: [
              { name: 'blueprintId', type: 'uint64', indexed: true },
              { name: 'operator', type: 'address', indexed: true },
              { name: 'ecdsaPublicKey', type: 'bytes', indexed: false },
              { name: 'rpcAddress', type: 'string', indexed: false },
            ],
          },
          args: { blueprintId },
          fromBlock: 0n,
          toBlock: 'latest',
        });

        if (cancelled) return;

        // Deduplicate by address (keep latest)
        const byAddress = new Map<Address, DiscoveredOperator>();
        for (const log of registeredLogs) {
          const addr = log.args.operator as Address | undefined;
          if (!addr) continue;
          byAddress.set(addr, {
            address: addr,
            ecdsaPublicKey: (log.args.ecdsaPublicKey as string) ?? '0x',
            rpcAddress: (log.args.rpcAddress as string) ?? '',
          });
        }

        const candidates = Array.from(byAddress.values());
        if (candidates.length === 0) {
          setOperators([]);
          setIsLoading(false);
          return;
        }

        // Step 3: Verify each is still registered + fetch current preferences
        const [registrationResults, preferencesResults] = await Promise.all([
          publicClient.multicall({
            contracts: candidates.map((op) => ({
              address: addrs.services,
              abi: tangleOperatorsAbi,
              functionName: 'isOperatorRegistered' as const,
              args: [blueprintId, op.address] as const,
            })),
          }),
          publicClient.multicall({
            contracts: candidates.map((op) => ({
              address: addrs.services,
              abi: tangleOperatorsAbi,
              functionName: 'getOperatorPreferences' as const,
              args: [blueprintId, op.address] as const,
            })),
          }),
        ]);

        if (cancelled) return;

        const active: DiscoveredOperator[] = [];
        candidates.forEach((op, i) => {
          if (registrationResults[i]?.result !== true) return;
          const prefs = preferencesResults[i];
          if (prefs?.status === 'success' && prefs.result != null) {
            const result = prefs.result as Record<string, unknown>;
            const ecdsaKey = Array.isArray(result) ? String(result[0] ?? '') : String((result as any).ecdsaPublicKey ?? '');
            const rpcAddr = Array.isArray(result) ? String(result[1] ?? '') : String((result as any).rpcAddress ?? '');
            active.push({
              ...op,
              ecdsaPublicKey: ecdsaKey || op.ecdsaPublicKey,
              rpcAddress: rpcAddr || op.rpcAddress,
            });
          } else {
            active.push(op);
          }
        });

        setOperators(active);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err : new Error(String(err)));
        }
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    }

    discover();
    return () => { cancelled = true; };
  }, [blueprintId]);

  return { operators, isLoading, error, operatorCount };
}
