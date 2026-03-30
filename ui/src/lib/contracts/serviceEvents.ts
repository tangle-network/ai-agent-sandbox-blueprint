import { decodeEventLog } from 'viem';
import { tangleServicesAbi } from '@tangle-network/blueprint-ui';

interface EventLogLike {
  data: `0x${string}`;
  topics: readonly `0x${string}`[];
}

// The current service manager emits ServiceRequested as:
//   ServiceRequested(uint64 requestId, uint64 blueprintId, address requester)
// Keep a raw-topic fallback so recovery still works even if a linked package
// build is temporarily out of sync with the chain ABI.
export const SERVICE_REQUESTED_TOPIC =
  '0xbd1fdda393b679e6c4f873e233b34e2c4ea8283a3f76345dbc143b86ea047679';

export function extractServiceRequestId(log: EventLogLike): number | null {
  try {
    const decoded = decodeEventLog({
      abi: tangleServicesAbi,
      data: log.data,
      topics: [...log.topics] as [] | [`0x${string}`, ...`0x${string}`[]],
    });
    if (decoded.eventName === 'ServiceRequested' && 'requestId' in decoded.args) {
      return Number(decoded.args.requestId);
    }
  } catch {
    // Fall back to raw topic parsing below.
  }

  const [topic0, topic1] = log.topics;
  if (!topic0 || !topic1) return null;
  if (topic0.toLowerCase() !== SERVICE_REQUESTED_TOPIC) return null;

  try {
    return Number(BigInt(topic1));
  } catch {
    return null;
  }
}
