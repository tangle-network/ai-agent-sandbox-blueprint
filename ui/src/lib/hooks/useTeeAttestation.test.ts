import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useTeeAttestation } from './useTeeAttestation';

type OperatorApiCall = (
  action: string,
  body?: Record<string, unknown>,
  opts?: { method?: string },
) => Promise<Response>;

let mockApiCall: ReturnType<typeof vi.fn<OperatorApiCall>>;

beforeEach(() => {
  mockApiCall = vi.fn();
});

function setup() {
  return renderHook(() => useTeeAttestation(mockApiCall));
}

const MOCK_ATTESTATION = {
  tee_type: 'TDX',
  evidence: [1, 2, 3],
  measurement: [10, 20, 30],
  timestamp: 1700000000,
};

describe('initial state', () => {
  it('starts with no attestation, not busy, no error', () => {
    const { result } = setup();
    expect(result.current.attestation).toBeNull();
    expect(result.current.busy).toBe(false);
    expect(result.current.error).toBeNull();
  });
});

describe('successful fetch', () => {
  it('sets attestation data on success', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify(MOCK_ATTESTATION)));
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });

    expect(result.current.attestation).toEqual(MOCK_ATTESTATION);
    expect(result.current.busy).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('calls operatorApiCall with correct args', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify(MOCK_ATTESTATION)));
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });

    expect(mockApiCall).toHaveBeenCalledWith('tee/attestation', undefined, { method: 'GET' });
  });
});

describe('error handling', () => {
  it('sets error string on Error throw', async () => {
    mockApiCall.mockRejectedValue(new Error('Network timeout'));
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });

    expect(result.current.attestation).toBeNull();
    expect(result.current.error).toBe('Network timeout');
    expect(result.current.busy).toBe(false);
  });

  it('sets fallback error for non-Error throw', async () => {
    mockApiCall.mockRejectedValue('string error');
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });

    expect(result.current.error).toBe('Failed to fetch attestation');
  });

  it('clears previous error on retry', async () => {
    mockApiCall
      .mockRejectedValueOnce(new Error('First fail'))
      .mockResolvedValueOnce(new Response(JSON.stringify(MOCK_ATTESTATION)));
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });
    expect(result.current.error).toBe('First fail');

    await act(async () => {
      await result.current.fetchAttestation();
    });
    expect(result.current.error).toBeNull();
    expect(result.current.attestation).toEqual(MOCK_ATTESTATION);
  });
});

describe('busy state', () => {
  it('is false after success', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify(MOCK_ATTESTATION)));
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });

    expect(result.current.busy).toBe(false);
  });

  it('is false after error', async () => {
    mockApiCall.mockRejectedValue(new Error('fail'));
    const { result } = setup();

    await act(async () => {
      await result.current.fetchAttestation();
    });

    expect(result.current.busy).toBe(false);
  });
});
