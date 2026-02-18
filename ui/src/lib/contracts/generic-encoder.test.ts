import { describe, it, expect } from 'vitest';
import { decodeAbiParameters } from 'viem';
import { encodeJobArgs } from './generic-encoder';
import type { JobDefinition } from '~/lib/blueprints/registry';

const makeJob = (overrides: Partial<JobDefinition> = {}): JobDefinition => ({
  id: 99,
  name: 'test_job',
  label: 'Test Job',
  description: 'Test',
  category: 'execution',
  icon: 'i-ph:test',
  pricingMultiplier: 1,
  requiresSandbox: false,
  fields: [],
  ...overrides,
});

describe('encodeJobArgs', () => {
  it('encodes string fields', () => {
    const job = makeJob({
      fields: [
        { name: 'command', label: 'Command', type: 'text', abiType: 'string' },
      ],
    });
    const encoded = encodeJobArgs(job, { command: 'ls -la' });
    const [decoded] = decodeAbiParameters(
      [{ name: 'command', type: 'string' }],
      encoded,
    );
    expect(decoded).toBe('ls -la');
  });

  it('encodes uint64 fields as BigInt', () => {
    const job = makeJob({
      fields: [
        { name: 'timeout', label: 'Timeout', type: 'number', abiType: 'uint64' },
      ],
    });
    const encoded = encodeJobArgs(job, { timeout: 30000 });
    const [decoded] = decodeAbiParameters(
      [{ name: 'timeout', type: 'uint64' }],
      encoded,
    );
    expect(decoded).toBe(30000n);
  });

  it('encodes bool fields', () => {
    const job = makeJob({
      fields: [
        { name: 'enabled', label: 'Enabled', type: 'boolean', abiType: 'bool' },
      ],
    });
    const encoded = encodeJobArgs(job, { enabled: true });
    const [decoded] = decodeAbiParameters(
      [{ name: 'enabled', type: 'bool' }],
      encoded,
    );
    expect(decoded).toBe(true);
  });

  it('prepends context params before form fields', () => {
    const job = makeJob({
      contextParams: [{ abiName: 'sidecar_url', abiType: 'string' }],
      fields: [
        { name: 'command', label: 'Command', type: 'text', abiType: 'string' },
      ],
    });
    const encoded = encodeJobArgs(
      job,
      { command: 'whoami' },
      { sidecar_url: 'http://localhost:8080' },
    );
    const [url, cmd] = decodeAbiParameters(
      [
        { name: 'sidecar_url', type: 'string' },
        { name: 'command', type: 'string' },
      ],
      encoded,
    );
    expect(url).toBe('http://localhost:8080');
    expect(cmd).toBe('whoami');
  });

  it('uses abiParam for field name when specified', () => {
    const job = makeJob({
      fields: [
        {
          name: 'agentId',
          label: 'Agent ID',
          type: 'text',
          abiType: 'string',
          abiParam: 'agent_identifier',
        },
      ],
    });
    const encoded = encodeJobArgs(job, { agentId: 'agent-1' });
    const [decoded] = decodeAbiParameters(
      [{ name: 'agent_identifier', type: 'string' }],
      encoded,
    );
    expect(decoded).toBe('agent-1');
  });

  it('skips fields without abiType', () => {
    const job = makeJob({
      fields: [
        { name: 'display', label: 'Display', type: 'text' }, // no abiType
        { name: 'command', label: 'Command', type: 'text', abiType: 'string' },
      ],
    });
    const encoded = encodeJobArgs(job, { display: 'ignored', command: 'test' });
    const [decoded] = decodeAbiParameters(
      [{ name: 'command', type: 'string' }],
      encoded,
    );
    expect(decoded).toBe('test');
  });

  it('coerces string[] from newline-separated text', () => {
    const job = makeJob({
      fields: [
        { name: 'urls', label: 'URLs', type: 'textarea', abiType: 'string[]' },
      ],
    });
    const encoded = encodeJobArgs(job, { urls: 'http://a\nhttp://b\nhttp://c' });
    const [decoded] = decodeAbiParameters(
      [{ name: 'urls', type: 'string[]' }],
      encoded,
    );
    expect(decoded).toEqual(['http://a', 'http://b', 'http://c']);
  });

  it('delegates to customEncoder when present', () => {
    const job = makeJob({
      customEncoder: (values) =>
        '0x1234' as `0x${string}`,
      fields: [
        { name: 'ignored', label: 'Ignored', type: 'text', abiType: 'string' },
      ],
    });
    const encoded = encodeJobArgs(job, { ignored: 'test' });
    expect(encoded).toBe('0x1234');
  });

  it('handles missing values gracefully', () => {
    const job = makeJob({
      fields: [
        { name: 'name', label: 'Name', type: 'text', abiType: 'string' },
        { name: 'count', label: 'Count', type: 'number', abiType: 'uint64' },
        { name: 'flag', label: 'Flag', type: 'boolean', abiType: 'bool' },
      ],
    });
    const encoded = encodeJobArgs(job, {}); // no values
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
});
