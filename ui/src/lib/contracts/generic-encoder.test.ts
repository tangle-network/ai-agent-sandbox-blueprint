import { describe, it, expect } from 'vitest';
import { decodeAbiParameters } from 'viem';
import { encodeJobArgs } from './generic-encoder';
import { makeJob, makeField } from '~/test/fixtures';

// ── Unit tests: coercion, context params, edge cases ──

describe('encodeJobArgs', () => {
  it('encodes string fields', () => {
    const job = makeJob({
      fields: [makeField({ name: 'command', type: 'text', abiType: 'string' })],
    });
    const encoded = encodeJobArgs(job, { command: 'ls -la' });
    const [decoded] = decodeAbiParameters([{ name: 'command', type: 'string' }], encoded);
    expect(decoded).toBe('ls -la');
  });

  it('encodes uint64 fields as BigInt', () => {
    const job = makeJob({
      fields: [makeField({ name: 'timeout', type: 'number', abiType: 'uint64' })],
    });
    const encoded = encodeJobArgs(job, { timeout: 30000 });
    const [decoded] = decodeAbiParameters([{ name: 'timeout', type: 'uint64' }], encoded);
    expect(decoded).toBe(30000n);
  });

  it('encodes uint8 fields as Number', () => {
    const job = makeJob({
      fields: [makeField({ name: 'teeType', type: 'select', abiType: 'uint8' })],
    });
    const encoded = encodeJobArgs(job, { teeType: '2' });
    const [decoded] = decodeAbiParameters([{ name: 'tee_type', type: 'uint8' }], encoded);
    expect(decoded).toBe(2);
  });

  it('encodes uint32 fields as Number', () => {
    const job = makeJob({
      fields: [makeField({ name: 'count', type: 'number', abiType: 'uint32' })],
    });
    const encoded = encodeJobArgs(job, { count: 5 });
    const [decoded] = decodeAbiParameters([{ name: 'count', type: 'uint32' }], encoded);
    expect(decoded).toBe(5);
  });

  it('encodes bool fields', () => {
    const job = makeJob({
      fields: [makeField({ name: 'enabled', type: 'boolean', abiType: 'bool' })],
    });
    const encoded = encodeJobArgs(job, { enabled: true });
    const [decoded] = decodeAbiParameters([{ name: 'enabled', type: 'bool' }], encoded);
    expect(decoded).toBe(true);
  });

  it('coerces falsy values to false for bool', () => {
    const job = makeJob({
      fields: [makeField({ name: 'flag', type: 'boolean', abiType: 'bool' })],
    });
    const encoded = encodeJobArgs(job, { flag: '' });
    const [decoded] = decodeAbiParameters([{ name: 'flag', type: 'bool' }], encoded);
    expect(decoded).toBe(false);
  });

  it('prepends context params before form fields', () => {
    const job = makeJob({
      contextParams: [{ abiName: 'sidecar_url', abiType: 'string' }],
      fields: [makeField({ name: 'command', type: 'text', abiType: 'string' })],
    });
    const encoded = encodeJobArgs(job, { command: 'whoami' }, { sidecar_url: 'http://localhost:8080' });
    const [url, cmd] = decodeAbiParameters(
      [{ name: 'sidecar_url', type: 'string' }, { name: 'command', type: 'string' }],
      encoded,
    );
    expect(url).toBe('http://localhost:8080');
    expect(cmd).toBe('whoami');
  });

  it('uses abiParam for ABI name when specified', () => {
    const job = makeJob({
      fields: [makeField({ name: 'agentId', type: 'text', abiType: 'string', abiParam: 'agent_identifier' })],
    });
    const encoded = encodeJobArgs(job, { agentId: 'agent-1' });
    const [decoded] = decodeAbiParameters([{ name: 'agent_identifier', type: 'string' }], encoded);
    expect(decoded).toBe('agent-1');
  });

  it('skips fields without abiType', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'display', type: 'text' }), // no abiType
        makeField({ name: 'command', type: 'text', abiType: 'string' }),
      ],
    });
    const encoded = encodeJobArgs(job, { display: 'ignored', command: 'test' });
    const [decoded] = decodeAbiParameters([{ name: 'command', type: 'string' }], encoded);
    expect(decoded).toBe('test');
  });

  it('coerces string[] from newline-separated text', () => {
    const job = makeJob({
      fields: [makeField({ name: 'urls', type: 'textarea', abiType: 'string[]' })],
    });
    const encoded = encodeJobArgs(job, { urls: 'http://a\nhttp://b\nhttp://c' });
    const [decoded] = decodeAbiParameters([{ name: 'urls', type: 'string[]' }], encoded);
    expect(decoded).toEqual(['http://a', 'http://b', 'http://c']);
  });

  it('passes through string[] when already an array', () => {
    const job = makeJob({
      fields: [makeField({ name: 'urls', type: 'textarea', abiType: 'string[]' })],
    });
    const encoded = encodeJobArgs(job, { urls: ['http://a', 'http://b'] });
    const [decoded] = decodeAbiParameters([{ name: 'urls', type: 'string[]' }], encoded);
    expect(decoded).toEqual(['http://a', 'http://b']);
  });

  it('filters empty lines from string[] coercion', () => {
    const job = makeJob({
      fields: [makeField({ name: 'urls', type: 'textarea', abiType: 'string[]' })],
    });
    const encoded = encodeJobArgs(job, { urls: 'http://a\n\nhttp://b\n' });
    const [decoded] = decodeAbiParameters([{ name: 'urls', type: 'string[]' }], encoded);
    expect(decoded).toEqual(['http://a', 'http://b']);
  });

  it('validates address[] format', () => {
    const job = makeJob({
      fields: [makeField({ name: 'ops', type: 'textarea', abiType: 'address[]' })],
    });
    const encoded = encodeJobArgs(job, {
      ops: '0x1234567890abcdef1234567890abcdef12345678\ninvalid\n0xabcdefabcdefabcdefabcdefabcdefabcdefabcd',
    });
    const [decoded] = decodeAbiParameters([{ name: 'ops', type: 'address[]' }], encoded);
    expect(decoded).toHaveLength(2);
    expect((decoded as string[])[0].toLowerCase()).toBe('0x1234567890abcdef1234567890abcdef12345678');
  });

  it('delegates to customEncoder when present', () => {
    const job = makeJob({
      customEncoder: () => '0x1234' as `0x${string}`,
      fields: [makeField({ name: 'ignored', type: 'text', abiType: 'string' })],
    });
    expect(encodeJobArgs(job, { ignored: 'test' })).toBe('0x1234');
  });

  it('passes both formValues and context to customEncoder', () => {
    const job = makeJob({
      customEncoder: (values, ctx) =>
        `0x${String(values.a)}${String(ctx?.b)}` as `0x${string}`,
    });
    expect(encodeJobArgs(job, { a: '11' }, { b: '22' })).toBe('0x1122');
  });

  it('handles missing values gracefully', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'name', type: 'text', abiType: 'string' }),
        makeField({ name: 'count', type: 'number', abiType: 'uint64' }),
        makeField({ name: 'flag', type: 'boolean', abiType: 'bool' }),
      ],
    });
    const encoded = encodeJobArgs(job, {});
    const decoded = decodeAbiParameters(
      [
        { name: 'name', type: 'string' },
        { name: 'count', type: 'uint64' },
        { name: 'flag', type: 'bool' },
      ],
      encoded,
    );
    expect(decoded[0]).toBe('');
    expect(decoded[1]).toBe(0n);
    expect(decoded[2]).toBe(false);
  });

  it('handles missing context gracefully', () => {
    const job = makeJob({
      contextParams: [{ abiName: 'sidecar_url', abiType: 'string' }],
      fields: [makeField({ name: 'cmd', type: 'text', abiType: 'string' })],
    });
    // No context passed — should default to empty string
    const encoded = encodeJobArgs(job, { cmd: 'test' });
    const [url, cmd] = decodeAbiParameters(
      [{ name: 'sidecar_url', type: 'string' }, { name: 'cmd', type: 'string' }],
      encoded,
    );
    expect(url).toBe('');
    expect(cmd).toBe('test');
  });

  it('encodes multiple fields in definition order', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'a', type: 'text', abiType: 'string' }),
        makeField({ name: 'b', type: 'number', abiType: 'uint64' }),
        makeField({ name: 'c', type: 'boolean', abiType: 'bool' }),
        makeField({ name: 'd', type: 'text', abiType: 'string' }),
      ],
    });
    const encoded = encodeJobArgs(job, { a: 'hello', b: 42, c: true, d: 'world' });
    const decoded = decodeAbiParameters(
      [
        { name: 'a', type: 'string' },
        { name: 'b', type: 'uint64' },
        { name: 'c', type: 'bool' },
        { name: 'd', type: 'string' },
      ],
      encoded,
    );
    expect(decoded[0]).toBe('hello');
    expect(decoded[1]).toBe(42n);
    expect(decoded[2]).toBe(true);
    expect(decoded[3]).toBe('world');
  });
});
