import { useState, useCallback } from 'react';
import type { Address } from 'viem';
import { tangleServicesAbi } from '~/lib/contracts/abi';
import { publicClient, getAddresses } from '~/lib/contracts/publicClient';

export interface ServiceInfo {
  active: boolean;
  blueprintId: bigint;
  owner: Address;
  operatorCount: number;
  operators: Address[];
  permitted: boolean;
  ttl: bigint;
  createdAt: bigint;
}

/**
 * Validate a service on-chain: check if active, fetch operators,
 * verify the current user is a permitted caller.
 */
export function useServiceValidation() {
  const [isValidating, setIsValidating] = useState(false);
  const [serviceInfo, setServiceInfo] = useState<ServiceInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const validate = useCallback(async (serviceId: bigint, userAddress?: Address) => {
    setIsValidating(true);
    setError(null);
    setServiceInfo(null);

    const addrs = getAddresses();

    try {
      // Run reads in parallel without multicall to avoid type union issues
      const [isActiveResult, serviceDataResult, operatorsResult, permittedResult] = await Promise.all([
        publicClient.readContract({
          address: addrs.services,
          abi: tangleServicesAbi,
          functionName: 'isServiceActive',
          args: [serviceId],
        }).catch(() => false),

        publicClient.readContract({
          address: addrs.services,
          abi: tangleServicesAbi,
          functionName: 'getService',
          args: [serviceId],
        }).catch(() => null),

        publicClient.readContract({
          address: addrs.services,
          abi: tangleServicesAbi,
          functionName: 'getServiceOperators',
          args: [serviceId],
        }).catch(() => [] as readonly Address[]),

        userAddress
          ? publicClient.readContract({
              address: addrs.services,
              abi: tangleServicesAbi,
              functionName: 'isPermittedCaller',
              args: [serviceId, userAddress],
            }).catch(() => false)
          : Promise.resolve(true),
      ]);

      const isActive = isActiveResult as boolean;
      const serviceData = serviceDataResult as Record<string, any> | null;
      const operators = (operatorsResult ?? []) as readonly Address[];
      const permitted = permittedResult as boolean;

      if (!serviceData) {
        setError('Service not found');
        setIsValidating(false);
        return null;
      }

      const info: ServiceInfo = {
        active: isActive,
        blueprintId: serviceData.blueprintId ?? serviceData[0] ?? 0n,
        owner: serviceData.owner ?? serviceData[1] ?? ('0x0' as Address),
        operatorCount: operators.length,
        operators: [...operators],
        permitted,
        ttl: serviceData.ttl ?? serviceData[3] ?? 0n,
        createdAt: serviceData.createdAt ?? serviceData[2] ?? 0n,
      };

      setServiceInfo(info);

      if (!isActive) {
        setError('Service is not active');
      } else if (!permitted && userAddress) {
        setError('You are not a permitted caller for this service');
      }

      setIsValidating(false);
      return info;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setIsValidating(false);
      return null;
    }
  }, []);

  const reset = useCallback(() => {
    setServiceInfo(null);
    setError(null);
  }, []);

  return { validate, reset, isValidating, serviceInfo, error };
}
