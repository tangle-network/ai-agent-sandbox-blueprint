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
  it('returns accepted run metadata for proxied prompt requests', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          accepted: true,
          run_id: 'run-1',
          session_id: 'session-1',
          status: 'queued',
          accepted_at: 123,
        }),
        { status: 202 },
      ),
    );

    const client = createProxiedClient('sandbox-1', async () => 'op-token-1', 'http://operator:9090');
    const result = await client.prompt('hello world');

    expect(result.accepted).toBe(true);
    expect(result.runId).toBe('run-1');
    expect(result.sessionId).toBe('session-1');
    expect(result.status).toBe('queued');

    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-1/prompt');
    expect(JSON.parse(options.body as string)).toEqual({ message: 'hello world' });
  });

  it('returns accepted run metadata for proxied task requests', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          accepted: true,
          run_id: 'run-task-1',
          session_id: 'chat-7',
          status: 'queued',
          accepted_at: 456,
        }),
        { status: 202 },
      ),
    );

    const client = createProxiedClient('sandbox-2', async () => 'op-token-2', 'http://operator:9090');
    const result = await client.task('build', undefined, 'chat-7');

    expect(result.runId).toBe('run-task-1');
    expect(result.sessionId).toBe('chat-7');
    expect(result.status).toBe('queued');

    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-2/task');
    expect(JSON.parse(options.body as string)).toEqual({ prompt: 'build', session_id: 'chat-7' });
  });

  it('targets /api/sandbox/* for proxied instance prompt requests', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          accepted: true,
          run_id: 'inst-run',
          session_id: 'inst-session',
          status: 'queued',
          accepted_at: 789,
        }),
        { status: 202 },
      ),
    );

    const client = createProxiedInstanceClient(async () => 'inst-token', 'http://instance-op:9091');
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

    const client = createProxiedClient('sandbox-3', async () => 'op-token-3', 'http://operator:9090');
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
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe('http://sidecar:8080/agent/prompt');
    expect(JSON.parse(options.body as string)).toEqual({ prompt: 'hello' });
  });

  it('throws when calling chat session methods in direct mode', async () => {
    const client = createDirectClient('http://sidecar:8080', 'sidecar-token');
    await expect(client.listChatSessions()).rejects.toThrow('only available in proxied mode');
  });
});

describe('SandboxClient chat session CRUD (sandbox-scoped)', () => {
  const makeClient = () =>
    createProxiedClient('sandbox-1', async () => 'op-token', 'http://operator:9090');

  it('listChatSessions hits the correct URL', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ sessions: [{ session_id: 's1', title: 'Chat', active_run_id: 'run-1' }] }), { status: 200 }),
    );

    const client = makeClient();
    const result = await client.listChatSessions();

    expect(result).toEqual([{ session_id: 's1', title: 'Chat', active_run_id: 'run-1' }]);
    expect(fetchMock.mock.calls[0][0]).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions');
  });

  it('createChatSession sends title in body', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ session_id: 'new-s', title: 'My Chat' }), { status: 200 }),
    );

    const client = makeClient();
    const result = await client.createChatSession('My Chat');

    expect(result.session_id).toBe('new-s');
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions');
    expect(JSON.parse(options.body as string)).toEqual({ title: 'My Chat' });
  });

  it('getChatSession returns messages and runs', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          session_id: 's1',
          title: 'T',
          active_run_id: 'run-1',
          messages: [{ id: 'm1', role: 'user', content: 'hello', created_at: 1 }],
          runs: [{ id: 'run-1', session_id: 's1', kind: 'prompt', status: 'running', request_text: 'hello', created_at: 1 }],
        }),
        { status: 200 },
      ),
    );

    const client = makeClient();
    const result = await client.getChatSession('s1');

    expect(result.session_id).toBe('s1');
    expect(result.runs[0].id).toBe('run-1');
    expect(fetchMock.mock.calls[0][0]).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions/s1');
  });

  it('deleteChatSession sends DELETE', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ deleted: true, session_id: 's1' }), { status: 200 }),
    );

    const client = makeClient();
    await client.deleteChatSession('s1');

    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions/s1');
    expect(options.method).toBe('DELETE');
  });

  it('cancelChatRun posts to the run cancel endpoint', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          success: true,
          session_id: 's1',
          run_id: 'run-1',
          status: 'cancelled',
          cancelled_at: 999,
        }),
        { status: 200 },
      ),
    );

    const client = makeClient();
    const result = await client.cancelChatRun('s1', 'run-1');

    expect(result.status).toBe('cancelled');
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions/s1/runs/run-1/cancel');
    expect(options.method).toBe('POST');
  });

  it('streamChatSession parses event names and payloads from SSE frames', async () => {
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(
          new TextEncoder().encode(
            'event: run_started\ndata: {"id":"run-1","session_id":"s1","kind":"prompt","status":"running","request_text":"hello","created_at":1}\n\n',
          ),
        );
        controller.close();
      },
    });

    fetchMock.mockResolvedValueOnce(
      new Response(stream, {
        status: 200,
        headers: { 'Content-Type': 'text/event-stream' },
      }),
    );

    const client = makeClient();
    const events: Array<{ type: string; data: unknown }> = [];
    await client.streamChatSession('s1', {
      onEvent: (event) => events.push(event),
    });

    expect(events).toHaveLength(1);
    expect(events[0].type).toBe('run_started');
    expect((events[0].data as { id: string }).id).toBe('run-1');
  });
});

describe('SandboxClient chat session CRUD (instance-scoped)', () => {
  const makeClient = () =>
    createProxiedInstanceClient(async () => 'inst-token', 'http://instance-op:9091');

  it('uses /api/sandbox/live/chat/sessions for instance mode', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ sessions: [] }), { status: 200 }),
    );

    const client = makeClient();
    await client.listChatSessions();

    expect(fetchMock.mock.calls[0][0]).toBe('http://instance-op:9091/api/sandbox/live/chat/sessions');
  });
});
