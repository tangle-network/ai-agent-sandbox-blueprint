import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createDirectClient,
  createProxiedClient,
  createProxiedInstanceClient,
} from './sandboxClient';

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('SandboxClient proxied mode', () => {
  it('sends proxied prompt directly and stores returned sidecar session id', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ response: 'ok', session_id: 'ses-1' }), { status: 200 }),
    );

    const client = createProxiedClient(
      'sandbox-1',
      async () => 'op-token-1',
      'http://operator:9090',
    );
    const result = await client.prompt('hello world');

    expect(result.response).toBe('ok');
    expect(result.sessionId).toBe('ses-1');
    expect(fetchMock).toHaveBeenCalledTimes(1);

    const [promptUrl, promptOpts] = fetchMock.mock.calls[0];
    const promptBody = JSON.parse(promptOpts.body as string);
    expect(promptUrl).toBe('http://operator:9090/api/sandboxes/sandbox-1/prompt');
    expect(promptBody.message).toBe('hello world');
    expect(promptBody.session_id).toBeUndefined();
    expect(promptBody.prompt).toBeUndefined();
    expect(promptOpts.headers.Authorization).toBe('Bearer op-token-1');
  });

  it('uses prompt/result contract for proxied task and skips chat-session creation when session is provided', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ result: 'done', session_id: 'sess-a' }), {
        status: 200,
      }),
    );

    const client = createProxiedClient(
      'sandbox-2',
      async () => 'op-token-2',
      'http://operator:9090',
    );
    const result = await client.task('build', undefined, 'sess-a');

    expect(result.response).toBe('done');
    expect(result.sessionId).toBe('sess-a');
    expect(fetchMock).toHaveBeenCalledTimes(1);

    const [url, opts] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-2/task');
    const body = JSON.parse(opts.body as string);
    expect(body.prompt).toBe('build');
    expect(body.task).toBeUndefined();
    expect(body.session_id).toBe('sess-a');
  });

  it('targets /api/sandbox/* for proxied instance client', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ response: 'inst-ok' }), { status: 200 }),
    );

    const client = createProxiedInstanceClient(
      async () => 'inst-token',
      'http://instance-op:9091',
    );
    await client.prompt('instance hello');

    expect(fetchMock.mock.calls[0][0]).toBe('http://instance-op:9091/api/sandbox/prompt');
  });

  it('formats warmup failures into a user-friendly retryable message', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          error: 'Sandbox agent is still starting up. Please retry shortly.',
          code: 'AGENT_WARMING_UP',
          retry_after_ms: 1200,
        }),
        { status: 503 },
      ),
    );

    const client = createProxiedClient(
      'sandbox-3',
      async () => 'op-token-3',
      'http://operator:9090',
    );

    await expect(client.prompt('hello')).rejects.toThrow(
      'Prompt failed (503): Sandbox agent is still starting up. Please retry shortly. Retry in about 2s.',
    );
  });
});

describe('SandboxClient direct mode', () => {
  it('uses sidecar prompt contract in direct mode', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ response: 'ok', sessionId: 's-1' }), { status: 200 }),
    );

    const client = createDirectClient('http://sidecar:8080', 'sidecar-token');
    const result = await client.prompt('hello');

    expect(result.response).toBe('ok');
    expect(result.sessionId).toBe('s-1');
    const [url, opts] = fetchMock.mock.calls[0];
    expect(url).toBe('http://sidecar:8080/agent/prompt');
    const body = JSON.parse(opts.body as string);
    expect(body.prompt).toBe('hello');
    expect(body.message).toBeUndefined();
  });

  it('throws when calling chat session methods in direct mode', async () => {
    const client = createDirectClient('http://sidecar:8080', 'sidecar-token');
    await expect(client.listChatSessions()).rejects.toThrow('only available in proxied mode');
  });
});

// ── Chat session CRUD ──

describe('SandboxClient chat session CRUD (sandbox-scoped)', () => {
  const makeClient = () =>
    createProxiedClient('sandbox-1', async () => 'op-token', 'http://operator:9090');

  it('listChatSessions hits correct URL', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ sessions: [{ session_id: 's1', title: 'Chat' }] }), { status: 200 }),
    );

    const client = makeClient();
    const result = await client.listChatSessions();

    expect(result).toEqual([{ session_id: 's1', title: 'Chat' }]);
    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions',
    );
  });

  it('createChatSession sends title in body', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ session_id: 'new-s', title: 'My Chat' }), { status: 200 }),
    );

    const client = makeClient();
    const result = await client.createChatSession('My Chat');

    expect(result.session_id).toBe('new-s');
    const [url, opts] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions');
    expect(opts.method).toBe('POST');
    expect(JSON.parse(opts.body as string)).toEqual({ title: 'My Chat' });
  });

  it('getChatSession hits correct URL with session ID', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ session_id: 's1', title: 'T', messages: [] }), { status: 200 }),
    );

    const client = makeClient();
    const result = await client.getChatSession('s1');

    expect(result.session_id).toBe('s1');
    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions/s1',
    );
  });

  it('deleteChatSession sends DELETE', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ deleted: true, session_id: 's1' }), { status: 200 }),
    );

    const client = makeClient();
    await client.deleteChatSession('s1');

    const [url, opts] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions/s1');
    expect(opts.method).toBe('DELETE');
  });

  it('throws on list failure', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response('Unauthorized', { status: 401 }),
    );

    const client = makeClient();
    await expect(client.listChatSessions()).rejects.toThrow('List chat sessions failed (401)');
  });
});

describe('SandboxClient chat session CRUD (instance-scoped)', () => {
  const makeClient = () =>
    createProxiedInstanceClient(async () => 'inst-token', 'http://instance-op:9091');

  it('listChatSessions uses /api/sandbox/ path', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ sessions: [] }), { status: 200 }),
    );

    const client = makeClient();
    await client.listChatSessions();

    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://instance-op:9091/api/sandbox/live/chat/sessions',
    );
  });

  it('createChatSession uses /api/sandbox/ path', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ session_id: 'i1', title: 'New Chat' }), { status: 200 }),
    );

    const client = makeClient();
    await client.createChatSession();

    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://instance-op:9091/api/sandbox/live/chat/sessions',
    );
  });

  it('getChatSession uses /api/sandbox/ path', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ session_id: 'i1', title: 'T', messages: [] }), { status: 200 }),
    );

    const client = makeClient();
    await client.getChatSession('i1');

    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://instance-op:9091/api/sandbox/live/chat/sessions/i1',
    );
  });

  it('deleteChatSession uses /api/sandbox/ path', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ deleted: true }), { status: 200 }),
    );

    const client = makeClient();
    await client.deleteChatSession('i1');

    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://instance-op:9091/api/sandbox/live/chat/sessions/i1',
    );
  });
});
