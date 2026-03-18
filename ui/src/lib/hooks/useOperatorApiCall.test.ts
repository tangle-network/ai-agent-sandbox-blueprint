import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useOperatorApiCall } from './useOperatorApiCall';

const OPERATOR_URL = 'http://test-operator:9090';
type GetTokenFn = (forceRefresh?: boolean) => Promise<string | null>;
type PathBuilderFn = (action: string) => string;

let mockGetToken: ReturnType<typeof vi.fn<GetTokenFn>>;
let mockBuildPath: ReturnType<typeof vi.fn<PathBuilderFn>>;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  mockGetToken = vi.fn().mockResolvedValue('test-token');
  mockBuildPath = vi.fn((action: string) => `/api/sandboxes/sb-1/${action}`);
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function setup() {
  return renderHook(() => useOperatorApiCall(OPERATOR_URL, mockGetToken, mockBuildPath));
}

function okResponse(body: unknown = {}) {
  return new Response(JSON.stringify(body), { status: 200 });
}

// ── Success path ──

describe('success path', () => {
  it('calls fetch with correct URL, auth header, and POST body', async () => {
    fetchMock.mockResolvedValue(okResponse({ ok: true }));
    const { result } = setup();

    await act(async () => {
      await result.current('stop', { force: true });
    });

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, opts] = fetchMock.mock.calls[0];
    expect(url).toBe('http://test-operator:9090/api/sandboxes/sb-1/stop');
    expect(opts.method).toBe('POST');
    expect(opts.headers.Authorization).toBe('Bearer test-token');
    expect(opts.body).toBe(JSON.stringify({ force: true }));
  });

  it('uses GET method when specified', async () => {
    fetchMock.mockResolvedValue(okResponse());
    const { result } = setup();

    await act(async () => {
      await result.current('ports', undefined, { method: 'GET' });
    });

    const [, opts] = fetchMock.mock.calls[0];
    expect(opts.method).toBe('GET');
    expect(opts.body).toBeUndefined();
    expect(opts.headers.Authorization).toBe('Bearer test-token');
    expect(opts.headers['Content-Type']).toBeUndefined();
  });

  it('uses HEAD without sending a body', async () => {
    fetchMock.mockResolvedValue(okResponse());
    const { result } = setup();

    await act(async () => {
      await result.current('status', undefined, { method: 'HEAD' });
    });

    const [, opts] = fetchMock.mock.calls[0];
    expect(opts.method).toBe('HEAD');
    expect(opts.body).toBeUndefined();
    expect(opts.headers.Authorization).toBe('Bearer test-token');
    expect(opts.headers['Content-Type']).toBeUndefined();
  });

  it('sends empty JSON body when no body provided', async () => {
    fetchMock.mockResolvedValue(okResponse());
    const { result } = setup();

    await act(async () => {
      await result.current('status');
    });

    expect(fetchMock.mock.calls[0][1].body).toBe('{}');
  });

  it('keeps JSON headers for POST requests', async () => {
    fetchMock.mockResolvedValue(okResponse());
    const { result } = setup();

    await act(async () => {
      await result.current('resume', { force: true });
    });

    const [, opts] = fetchMock.mock.calls[0];
    expect(opts.method).toBe('POST');
    expect(opts.body).toBe(JSON.stringify({ force: true }));
    expect(opts.headers['Content-Type']).toBe('application/json');
  });

  it('returns the Response on success', async () => {
    fetchMock.mockResolvedValue(okResponse({ data: 'test' }));
    const { result } = setup();

    let res: Response | undefined;
    await act(async () => {
      res = await result.current('status');
    });

    expect(res).toBeInstanceOf(Response);
    expect(res!.ok).toBe(true);
  });
});

// ── Auth failure ──

describe('auth failure', () => {
  it('throws when getToken returns null', async () => {
    mockGetToken.mockResolvedValue(null);
    const { result } = setup();

    await expect(
      act(async () => { await result.current('stop'); }),
    ).rejects.toThrow('Wallet authentication required');

    expect(fetchMock).not.toHaveBeenCalled();
  });
});

// ── 401 retry ──

describe('401 retry', () => {
  it('retries with fresh token on 401', async () => {
    fetchMock
      .mockResolvedValueOnce(new Response('Unauthorized', { status: 401 }))
      .mockResolvedValueOnce(okResponse({ retried: true }));
    mockGetToken
      .mockResolvedValueOnce('stale-token')
      .mockResolvedValueOnce('fresh-token');

    const { result } = setup();

    let res: Response | undefined;
    await act(async () => {
      res = await result.current('stop');
    });

    expect(mockGetToken).toHaveBeenCalledTimes(2);
    expect(mockGetToken).toHaveBeenLastCalledWith(true); // forceRefresh
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[1][1].headers.Authorization).toBe('Bearer fresh-token');
    expect(res!.ok).toBe(true);
  });

  it('throws when fresh token is also null', async () => {
    fetchMock.mockResolvedValue(new Response('Unauthorized', { status: 401 }));
    mockGetToken
      .mockResolvedValueOnce('stale-token')
      .mockResolvedValueOnce(null);

    const { result } = setup();

    await expect(
      act(async () => { await result.current('stop'); }),
    ).rejects.toThrow('Re-authentication failed');
  });
});

// ── Non-ok response ──

describe('non-ok response', () => {
  it('throws with action name, status code, and body text', async () => {
    fetchMock.mockResolvedValue(new Response('Not Found', { status: 404 }));
    const { result } = setup();

    await expect(
      act(async () => { await result.current('snapshot'); }),
    ).rejects.toThrow('snapshot failed (404): Not Found');
  });

  it('includes status 500 in error', async () => {
    fetchMock.mockResolvedValue(new Response('Internal error', { status: 500 }));
    const { result } = setup();

    await expect(
      act(async () => { await result.current('resume'); }),
    ).rejects.toThrow('resume failed (500): Internal error');
  });
});

// ── buildPath delegation ──

describe('buildPath', () => {
  it('uses buildPath to construct the URL', async () => {
    mockBuildPath.mockReturnValue('/api/sandbox/custom-path');
    fetchMock.mockResolvedValue(okResponse());
    const { result } = setup();

    await act(async () => {
      await result.current('custom-action');
    });

    expect(mockBuildPath).toHaveBeenCalledWith('custom-action');
    expect(fetchMock.mock.calls[0][0]).toBe('http://test-operator:9090/api/sandbox/custom-path');
  });
});
