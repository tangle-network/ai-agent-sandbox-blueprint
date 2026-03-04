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
  it('creates a live chat session on first proxied prompt and uses message/session_id payload', async () => {
    fetchMock
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ session_id: 'live-chat-1' }), { status: 200 }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ response: 'ok' }), { status: 200 }),
      );

    const client = createProxiedClient(
      'sandbox-1',
      async () => 'op-token-1',
      'http://operator:9090',
    );
    const result = await client.prompt('hello world');

    expect(result.response).toBe('ok');
    expect(result.sessionId).toBe('live-chat-1');
    expect(fetchMock).toHaveBeenCalledTimes(2);

    const [createUrl, createOpts] = fetchMock.mock.calls[0];
    expect(createUrl).toBe('http://operator:9090/api/sandboxes/sandbox-1/live/chat/sessions');
    expect(createOpts.method).toBe('POST');
    expect(createOpts.headers.Authorization).toBe('Bearer op-token-1');

    const [, promptOpts] = fetchMock.mock.calls[1];
    const promptBody = JSON.parse(promptOpts.body as string);
    expect(promptBody.message).toBe('hello world');
    expect(promptBody.session_id).toBe('live-chat-1');
    expect(promptBody.prompt).toBeUndefined();
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
    fetchMock
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ session_id: 'inst-live-1' }), { status: 200 }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ response: 'inst-ok' }), { status: 200 }),
      );

    const client = createProxiedInstanceClient(
      async () => 'inst-token',
      'http://instance-op:9091',
    );
    await client.prompt('instance hello');

    expect(fetchMock.mock.calls[0][0]).toBe(
      'http://instance-op:9091/api/sandbox/live/chat/sessions',
    );
    expect(fetchMock.mock.calls[1][0]).toBe('http://instance-op:9091/api/sandbox/prompt');
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
});
