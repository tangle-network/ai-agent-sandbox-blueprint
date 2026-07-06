import { decodeEventLog } from 'viem';
import { getAddresses, publicClient, tangleServicesAbi } from '@tangle-network/blueprint-ui';
import { extractServiceRequestId } from '~/lib/contracts/serviceEvents';

export type ServiceReceiptLog = {
  data: `0x${string}`;
  topics: readonly `0x${string}`[];
};

export function getRequestIdFromServiceReceiptLogs(logs: ServiceReceiptLog[]): number | null {
  for (const log of logs) {
    const requestId = extractServiceRequestId(log);
    if (requestId != null) return requestId;
  }

  return null;
}

export async function resolveActivatedServiceId(requestId: number): Promise<string | null> {
  const addrs = getAddresses();
  const logs = await publicClient.getLogs({
    address: addrs.services,
    fromBlock: 0n,
    toBlock: 'latest',
  });

  for (const log of logs) {
    try {
      const decoded = decodeEventLog({
        abi: tangleServicesAbi,
        data: log.data,
        topics: [...log.topics] as [] | [`0x${string}`, ...`0x${string}`[]],
      });
      if (decoded.eventName !== 'ServiceActivated') continue;
      if (!('requestId' in decoded.args) || !('serviceId' in decoded.args)) continue;
      if (Number(decoded.args.requestId) !== requestId) continue;
      return String(decoded.args.serviceId);
    } catch {
      // Ignore unrelated logs while scanning the service manager.
    }
  }

  return null;
}
