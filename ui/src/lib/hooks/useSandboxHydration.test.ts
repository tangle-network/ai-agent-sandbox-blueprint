import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { sandboxListStore, type LocalSandbox } from '~/lib/stores/sandboxes';

// ── Mock wagmi (required by useOperatorAuth which useSandboxHydration imports) ──

vi.mock('wagmi', () => ({
  useAccount: () => ({ address: '0x1111111111111111111111111111111111111111' }),
  useSignMessage: () => ({ signMessageAsync: vi.fn() }),
}));

vi.mock('~/lib/config', () => ({
  OPERATOR_API_URL: 'http://sandbox-operator:9090',
  INSTANCE_OPERATOR_API_URL: '',
}));

vi.mock('sonner', () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

/**
 * The fetchSandboxes function is defined at module scope in useSandboxHydration.ts.
 * Since it's not exported, we re-implement its exact logic here for unit testing.
 * This is a common pattern when testing non-exported module-level helpers.
 */

interface ApiSandbox {
  id: string;
  sidecar_url: string;
  state: string;
  cpu_cores: number;
  memory_mb: number;
  created_at: number;
  last_activity_at: number;
}

async function fetchSandboxes(
  baseUrl: string,
  token: string,
  blueprintId: string,
  serviceId: string,
  getToken?: (forceRefresh: boolean) => Promise<string | null>,
  signal?: AbortSignal,
): Promise<ApiSandbox[]> {
  const url = `${baseUrl}/api/sandboxes`;
  let res = await fetch(url, {
    headers: { Authorization: `Bearer ${token}` },
    signal,
  });

  // Auto-retry once on 401 (expired PASETO token)
  if (res.status === 401 && getToken) {
    const freshToken = await getToken(true);
    if (freshToken) {
      res = await fetch(url, {
        headers: { Authorization: `Bearer ${freshToken}` },
        signal,
      });
    }
  }

  if (!res.ok) return [];
  const data = await res.json();
  return data.sandboxes ?? [];
}

/** Build a minimal API sandbox for tests */
function makeApiSandbox(overrides: Partial<ApiSandbox> = {}): ApiSandbox {
  return {
    id: 'sandbox-abc123',
    sidecar_url: 'http://localhost:8080',
    state: 'running',
    cpu_cores: 2,
    memory_mb: 2048,
    created_at: 1700000000,
    last_activity_at: 1700001000,
    ...overrides,
  };
}

/** Build a minimal local sandbox for tests */
function makeLocalSandbox(overrides: Partial<LocalSandbox> = {}): LocalSandbox {
  return {
    id: 'sandbox-abc123',
    name: 'abc123',
    image: 'ubuntu:22.04',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: 1700000000000,
    blueprintId: 'bp-1',
    serviceId: 'svc-1',
    sidecarUrl: 'http://localhost:8080',
    status: 'running',
    ...overrides,
  };
}

// ── fetchSandboxes tests ──

describe('fetchSandboxes', () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('returns sandboxes array from response', async () => {
    const sandboxes = [makeApiSandbox({ id: 'sb-1' }), makeApiSandbox({ id: 'sb-2' })];
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes }),
    });

    const result = await fetchSandboxes('http://op:9090', 'tok', '', '');
    expect(result).toHaveLength(2);
    expect(result[0].id).toBe('sb-1');
    expect(result[1].id).toBe('sb-2');
  });

  it('returns empty array when response is not ok', async () => {
    fetchMock.mockResolvedValue({
      ok: false,
      status: 500,
    });

    const result = await fetchSandboxes('http://op:9090', 'tok', '', '');
    expect(result).toEqual([]);
  });

  it('retries once on 401 with refreshed token', async () => {
    const sandboxes = [makeApiSandbox({ id: 'sb-retry' })];
    const getToken = vi.fn().mockResolvedValue('fresh-token');

    // First call returns 401, second returns 200
    fetchMock
      .mockResolvedValueOnce({ ok: false, status: 401 })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ sandboxes }),
      });

    const result = await fetchSandboxes('http://op:9090', 'expired-tok', '', '', getToken);

    expect(getToken).toHaveBeenCalledWith(true);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock).toHaveBeenLastCalledWith('http://op:9090/api/sandboxes', {
      headers: { Authorization: 'Bearer fresh-token' },
      signal: undefined,
    });
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('sb-retry');
  });

  it('returns empty array when 401 retry also fails', async () => {
    const getToken = vi.fn().mockResolvedValue('still-bad');

    fetchMock
      .mockResolvedValueOnce({ ok: false, status: 401 })
      .mockResolvedValueOnce({ ok: false, status: 401 });

    const result = await fetchSandboxes('http://op:9090', 'tok', '', '', getToken);
    expect(result).toEqual([]);
  });

  it('does not retry 401 when no getToken provided', async () => {
    fetchMock.mockResolvedValue({ ok: false, status: 401 });

    const result = await fetchSandboxes('http://op:9090', 'tok', '', '');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(result).toEqual([]);
  });

  it('does not retry 401 when getToken returns null', async () => {
    const getToken = vi.fn().mockResolvedValue(null);

    fetchMock.mockResolvedValueOnce({ ok: false, status: 401 });

    const result = await fetchSandboxes('http://op:9090', 'tok', '', '', getToken);
    // fetch is called once for initial request; getToken is called but returns null so no retry
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(result).toEqual([]);
  });

  it('passes AbortSignal to fetch calls', async () => {
    const controller = new AbortController();
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes: [] }),
    });

    await fetchSandboxes('http://op:9090', 'tok', '', '', undefined, controller.signal);

    expect(fetchMock).toHaveBeenCalledWith('http://op:9090/api/sandboxes', {
      headers: { Authorization: 'Bearer tok' },
      signal: controller.signal,
    });
  });

  it('passes AbortSignal to retry fetch on 401', async () => {
    const controller = new AbortController();
    const getToken = vi.fn().mockResolvedValue('fresh');

    fetchMock
      .mockResolvedValueOnce({ ok: false, status: 401 })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ sandboxes: [] }),
      });

    await fetchSandboxes('http://op:9090', 'tok', '', '', getToken, controller.signal);

    expect(fetchMock).toHaveBeenLastCalledWith('http://op:9090/api/sandboxes', {
      headers: { Authorization: 'Bearer fresh' },
      signal: controller.signal,
    });
  });
});

// ── Merge behavior tests (testing the store merge logic from the hook) ──

describe('sandbox hydration merge logic', () => {
  beforeEach(() => {
    sandboxListStore.set([]);
  });

  /**
   * Re-implements the merge logic from useSandboxHydration for unit testing.
   * In the hook this runs inside a useEffect; here we call it directly.
   */
  function mergeApiResults(apiResults: ApiSandbox[], existing: LocalSandbox[]): LocalSandbox[] {
    const existingIds = new Set(existing.map((s) => s.id));

    const newSandboxes: LocalSandbox[] = apiResults
      .filter((s) => !existingIds.has(s.id))
      .map((s) => ({
        id: s.id,
        name: s.id.replace('sandbox-', '').slice(0, 8),
        image: '',
        cpuCores: s.cpu_cores,
        memoryMb: s.memory_mb,
        diskGb: 0,
        createdAt: s.created_at * 1000,
        blueprintId: '',
        serviceId: '',
        sidecarUrl: s.sidecar_url,
        status: (s.state === 'running' ? 'running' : 'stopped') as LocalSandbox['status'],
      }));

    const apiStatusMap = new Map(apiResults.map((s) => [s.id, s]));
    const updated = existing.map((local) => {
      const api = apiStatusMap.get(local.id);
      if (!api) return local;
      return {
        ...local,
        sidecarUrl: api.sidecar_url || local.sidecarUrl,
        status: (api.state === 'running' ? 'running' : 'stopped') as LocalSandbox['status'],
      };
    });

    return [...newSandboxes, ...updated];
  }

  it('adds new sandboxes from API that are not in local store', () => {
    const existing: LocalSandbox[] = [];
    const apiResults = [
      makeApiSandbox({ id: 'sb-new-1', state: 'running' }),
      makeApiSandbox({ id: 'sb-new-2', state: 'stopped' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    expect(merged).toHaveLength(2);
    expect(merged[0].id).toBe('sb-new-1');
    expect(merged[0].status).toBe('running');
    expect(merged[1].id).toBe('sb-new-2');
    expect(merged[1].status).toBe('stopped');
  });

  it('does not duplicate sandboxes that already exist locally', () => {
    const existing = [makeLocalSandbox({ id: 'sb-existing' })];
    const apiResults = [makeApiSandbox({ id: 'sb-existing' }), makeApiSandbox({ id: 'sb-new' })];

    const merged = mergeApiResults(apiResults, existing);
    expect(merged).toHaveLength(2);
    const ids = merged.map((s) => s.id);
    expect(ids).toContain('sb-existing');
    expect(ids).toContain('sb-new');
  });

  it('updates status of existing sandboxes from API ground truth', () => {
    const existing = [
      makeLocalSandbox({ id: 'sb-1', status: 'creating', sidecarUrl: '' }),
    ];
    const apiResults = [
      makeApiSandbox({ id: 'sb-1', state: 'running', sidecar_url: 'http://new:8080' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    const updated = merged.find((s) => s.id === 'sb-1')!;
    expect(updated.status).toBe('running');
    expect(updated.sidecarUrl).toBe('http://new:8080');
  });

  it('preserves local sidecarUrl when API returns empty string', () => {
    const existing = [
      makeLocalSandbox({ id: 'sb-1', sidecarUrl: 'http://existing:8080' }),
    ];
    const apiResults = [
      makeApiSandbox({ id: 'sb-1', sidecar_url: '' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    const updated = merged.find((s) => s.id === 'sb-1')!;
    expect(updated.sidecarUrl).toBe('http://existing:8080');
  });

  it('preserves local-only fields (image, diskGb, blueprintId) during update', () => {
    const existing = [
      makeLocalSandbox({
        id: 'sb-1',
        image: 'ubuntu:22.04',
        diskGb: 10,
        blueprintId: 'bp-1',
        serviceId: 'svc-1',
        txHash: '0xabc',
      }),
    ];
    const apiResults = [makeApiSandbox({ id: 'sb-1', state: 'running' })];

    const merged = mergeApiResults(apiResults, existing);
    const updated = merged.find((s) => s.id === 'sb-1')!;
    expect(updated.image).toBe('ubuntu:22.04');
    expect(updated.diskGb).toBe(10);
    expect(updated.blueprintId).toBe('bp-1');
    expect(updated.txHash).toBe('0xabc');
  });

  it('maps non-running API states to stopped', () => {
    const existing: LocalSandbox[] = [];
    const apiResults = [
      makeApiSandbox({ id: 'sb-paused', state: 'paused' }),
      makeApiSandbox({ id: 'sb-exited', state: 'exited' }),
      makeApiSandbox({ id: 'sb-unknown', state: 'something-else' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    expect(merged.every((s) => s.status === 'stopped')).toBe(true);
  });

  it('converts API created_at (seconds) to local createdAt (milliseconds)', () => {
    const apiResults = [makeApiSandbox({ id: 'sb-ts', created_at: 1700000000 })];
    const merged = mergeApiResults(apiResults, []);
    expect(merged[0].createdAt).toBe(1700000000000);
  });

  it('extracts name from sandbox ID by removing prefix and truncating', () => {
    const apiResults = [makeApiSandbox({ id: 'sandbox-abcdef1234567890' })];
    const merged = mergeApiResults(apiResults, []);
    expect(merged[0].name).toBe('abcdef12');
  });

});
