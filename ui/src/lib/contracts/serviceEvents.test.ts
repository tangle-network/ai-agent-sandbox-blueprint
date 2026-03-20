import { describe, expect, it } from 'vitest';
import { extractServiceRequestId, SERVICE_REQUESTED_TOPIC } from './serviceEvents';

describe('extractServiceRequestId', () => {
  it('falls back to the on-chain ServiceRequested topic layout', () => {
    const requestId = extractServiceRequestId({
      data: '0x',
      topics: [
        SERVICE_REQUESTED_TOPIC,
        '0x0000000000000000000000000000000000000000000000000000000000000003',
        '0x0000000000000000000000000000000000000000000000000000000000000002',
        '0x0000000000000000000000009965507d1a55bcc2695c58ba16fb37d819b0a4dc',
      ],
    });

    expect(requestId).toBe(3);
  });

  it('returns null for unrelated logs', () => {
    const requestId = extractServiceRequestId({
      data: '0x',
      topics: [
        '0x741e97ee1ff887c4d882f4c49ad280ea7d61d035e4e8a471e531951550275023',
        '0x0000000000000000000000000000000000000000000000000000000000000002',
      ],
    });

    expect(requestId).toBeNull();
  });
});
