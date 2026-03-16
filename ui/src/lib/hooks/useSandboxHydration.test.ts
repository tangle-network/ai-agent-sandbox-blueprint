import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { LocalSandbox } from '~/lib/stores/sandboxes';
import {
  fetchSandboxes,
  mergeApiResults,
  reconcileSandboxes,
  type ApiProvision,
  type ApiSandbox,
} from './sandboxHydrationLogic';

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
function makeLocalSandbox(overrides: Partial<LocalSandbox> & { id?: string } = {}): LocalSandbox {
  const { id, ...rest } = overrides;
  return {
    localId: id ?? 'sandbox-abc123',
    sandboxId: rest.sandboxId ?? (id && !id.startsWith('draft:') ? id : undefined),
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
    ...rest,
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

// ── Merge behavior tests ──

describe('sandbox hydration merge logic', () => {
  it('adds new sandboxes from API that are not in local store', () => {
    const existing: LocalSandbox[] = [];
    const apiResults = [
      makeApiSandbox({ id: 'sb-new-1', state: 'running' }),
      makeApiSandbox({ id: 'sb-new-2', state: 'stopped' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    expect(merged).toHaveLength(2);
    expect(merged[0].sandboxId).toBe('sb-new-1');
    expect(merged[0].status).toBe('running');
    expect(merged[1].sandboxId).toBe('sb-new-2');
    expect(merged[1].status).toBe('stopped');
  });

  it('does not duplicate sandboxes that already exist locally', () => {
    const existing = [makeLocalSandbox({ id: 'sb-existing' })];
    const apiResults = [makeApiSandbox({ id: 'sb-existing' }), makeApiSandbox({ id: 'sb-new' })];

    const merged = mergeApiResults(apiResults, existing);
    expect(merged).toHaveLength(2);
    const ids = merged.map((s) => s.sandboxId ?? s.localId);
    expect(ids).toContain('sb-existing');
    expect(ids).toContain('sb-new');
  });

  it('updates status of existing sandboxes from API ground truth', () => {
    const existing = [
      makeLocalSandbox({ id: 'sb-1', sandboxId: 'sb-1', status: 'creating', sidecarUrl: '' }),
    ];
    const apiResults = [
      makeApiSandbox({ id: 'sb-1', state: 'running', sidecar_url: 'http://new:8080' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    const updated = merged.find((s) => s.sandboxId === 'sb-1')!;
    expect(updated.status).toBe('running');
    expect(updated.sidecarUrl).toBe('http://new:8080');
  });

  it('preserves local sidecarUrl when API returns empty string', () => {
    const existing = [
      makeLocalSandbox({ id: 'sb-1', sandboxId: 'sb-1', sidecarUrl: 'http://existing:8080' }),
    ];
    const apiResults = [
      makeApiSandbox({ id: 'sb-1', sidecar_url: '' }),
    ];

    const merged = mergeApiResults(apiResults, existing);
    const updated = merged.find((s) => s.sandboxId === 'sb-1')!;
    expect(updated.sidecarUrl).toBe('http://existing:8080');
  });

  it('preserves local-only fields (image, diskGb, blueprintId) during update', () => {
    const existing = [
      makeLocalSandbox({
        id: 'sb-1',
        sandboxId: 'sb-1',
        image: 'ubuntu:22.04',
        diskGb: 10,
        blueprintId: 'bp-1',
        serviceId: 'svc-1',
        txHash: '0xabc',
      }),
    ];
    const apiResults = [makeApiSandbox({ id: 'sb-1', state: 'running' })];

    const merged = mergeApiResults(apiResults, existing);
    const updated = merged.find((s) => s.sandboxId === 'sb-1')!;
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

  it('attaches a canonical sandboxId to a pending draft when provision becomes ready', () => {
    const existing = [
      makeLocalSandbox({ id: 'draft:abc', status: 'creating', sandboxId: undefined, sidecarUrl: '' }),
    ];
    const provisions = new Map<number, ApiProvision | null>([
      [12, { call_id: 12, phase: 'ready', sandbox_id: 'sandbox-live-1', sidecar_url: 'http://live:8080' }],
    ]);

    const reconciled = reconcileSandboxes(
      [{ ...existing[0], callId: 12 }],
      [makeApiSandbox({ id: 'sandbox-live-1', state: 'running', sidecar_url: 'http://live:8080' })],
      provisions,
      { pruneUnverifiedDrafts: true, pruneMissingCanonical: true },
    );

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].localId).toBe('draft:abc');
    expect(reconciled[0].sandboxId).toBe('sandbox-live-1');
    expect(reconciled[0].status).toBe('running');
  });

  it('promotes a callId-free draft when exactly one backend sandbox fingerprint matches', () => {
    const createdAt = 1700000000000;
    const existing = [
      makeLocalSandbox({
        id: 'draft:recover-me',
        sandboxId: undefined,
        status: 'creating',
        sidecarUrl: '',
        name: 'sb1',
        image: 'agent-dev:latest',
        createdAt,
      }),
    ];

    const reconciled = reconcileSandboxes(
      existing,
      [
        makeApiSandbox({
          id: 'sandbox-live-1',
          name: 'sb1',
          image: 'agent-dev:latest',
          disk_gb: 10,
          created_at: createdAt / 1000,
          sidecar_url: 'http://live:8080',
        }),
      ],
      new Map(),
      { pruneUnverifiedDrafts: true, pruneMissingCanonical: true },
    );

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].localId).toBe('draft:recover-me');
    expect(reconciled[0].sandboxId).toBe('sandbox-live-1');
    expect(reconciled[0].status).toBe('running');
  });

  it('keeps a callId-free draft unchanged when backend fingerprint matching is ambiguous', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const createdAt = Date.now();
    const existing = [
      makeLocalSandbox({
        id: 'draft:ambiguous',
        sandboxId: undefined,
        status: 'creating',
        sidecarUrl: '',
        name: 'sb1',
        image: 'agent-dev:latest',
        createdAt,
        txHash: '0xabc',
      }),
    ];

    const reconciled = reconcileSandboxes(
      existing,
      [
        makeApiSandbox({
          id: 'sandbox-live-1',
          name: 'sb1',
          image: 'agent-dev:latest',
          disk_gb: 10,
          created_at: Math.floor(createdAt / 1000),
        }),
        makeApiSandbox({
          id: 'sandbox-live-2',
          name: 'sb1',
          image: 'agent-dev:latest',
          disk_gb: 10,
          created_at: Math.floor(createdAt / 1000),
        }),
      ],
      new Map(),
      { pruneUnverifiedDrafts: true, pruneMissingCanonical: true },
    );

    expect(reconciled).toHaveLength(3);
    expect(reconciled[0].localId).toBe('draft:ambiguous');
    expect(reconciled[0].sandboxId).toBeUndefined();
    expect(reconciled.slice(1).map((sandbox) => sandbox.sandboxId)).toEqual([
      'sandbox-live-1',
      'sandbox-live-2',
    ]);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    warnSpy.mockRestore();
  });

  it('removes stale drafts when operator truth is empty and no provision record exists', () => {
    const existing = [
      makeLocalSandbox({ id: 'draft:stale', sandboxId: undefined, status: 'creating', sidecarUrl: '' }),
    ];

    const reconciled = reconcileSandboxes(
      [{ ...existing[0], callId: 77 }],
      [],
      new Map([[77, null]]),
      { pruneUnverifiedDrafts: true, pruneMissingCanonical: true },
    );

    expect(reconciled).toHaveLength(0);
  });

  it('keeps failed drafts as error rows instead of removing them', () => {
    const existing = [
      makeLocalSandbox({ id: 'draft:failed', sandboxId: undefined, status: 'creating', sidecarUrl: '' }),
    ];

    const reconciled = reconcileSandboxes(
      [{ ...existing[0], callId: 88 }],
      [],
      new Map([[88, { call_id: 88, phase: 'failed', message: 'boom' }]]),
      { pruneUnverifiedDrafts: true, pruneMissingCanonical: true },
    );

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].status).toBe('error');
  });

  it('keeps recent tx-backed drafts while waiting for the receipt to yield a callId when operator truth is unavailable', () => {
    const now = Date.now();
    const existing = [
      makeLocalSandbox({
        id: 'draft:tx-pending',
        sandboxId: undefined,
        status: 'creating',
        sidecarUrl: '',
        txHash: '0xabc',
        createdAt: now,
      }),
    ];

    const reconciled = reconcileSandboxes(existing, [], new Map(), {
      pruneUnverifiedDrafts: false,
      pruneMissingCanonical: false,
    });

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].txHash).toBe('0xabc');
  });

  it('keeps recent tx-backed drafts after an authoritative operator refresh when no unique backend match exists', () => {
    const now = Date.now();
    const existing = [
      makeLocalSandbox({
        id: 'draft:tx-pending',
        sandboxId: undefined,
        status: 'creating',
        sidecarUrl: '',
        txHash: '0xabc',
        createdAt: now,
      }),
    ];

    const reconciled = reconcileSandboxes(existing, [makeApiSandbox({ id: 'sandbox-live-1' })], new Map(), {
      pruneUnverifiedDrafts: true,
      pruneMissingCanonical: true,
    });

    expect(reconciled).toHaveLength(2);
    expect(reconciled[0].localId).toBe('draft:tx-pending');
    expect(reconciled[0].sandboxId).toBeUndefined();
    expect(reconciled[1].sandboxId).toBe('sandbox-live-1');
  });

  it('prunes stale canonical sandboxes after a successful authoritative refresh', () => {
    const existing = [
      makeLocalSandbox({
        id: 'sandbox-stale',
        localId: 'canonical:sandbox-stale',
        sandboxId: 'sandbox-stale',
        status: 'stopped',
      }),
      makeLocalSandbox({
        id: 'sandbox-live-1',
        localId: 'canonical:sandbox-live-1',
        sandboxId: 'sandbox-live-1',
        status: 'running',
      }),
    ];

    const reconciled = reconcileSandboxes(existing, [makeApiSandbox({ id: 'sandbox-live-1' })], new Map(), {
      pruneUnverifiedDrafts: true,
      pruneMissingCanonical: true,
    });

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].sandboxId).toBe('sandbox-live-1');
  });
});
