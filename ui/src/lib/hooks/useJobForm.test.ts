import { describe, it, expect } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useJobForm } from './useJobForm';
import { makeJob, makeField } from '~/test/fixtures';
import { getJobById } from '~/lib/blueprints/registry';
import { JOB_IDS } from '~/lib/types/sandbox';
import { INSTANCE_JOB_IDS } from '~/lib/types/instance';
import '~/lib/blueprints'; // auto-register

// ── Defaults ──

describe('useJobForm defaults', () => {
  it('initializes text fields to empty string', () => {
    const job = makeJob({
      fields: [makeField({ name: 'cmd', type: 'text' })],
    });
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values.cmd).toBe('');
  });

  it('initializes number fields to 0', () => {
    const job = makeJob({
      fields: [makeField({ name: 'count', type: 'number' })],
    });
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values.count).toBe(0);
  });

  it('initializes boolean fields to false', () => {
    const job = makeJob({
      fields: [makeField({ name: 'enabled', type: 'boolean' })],
    });
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values.enabled).toBe(false);
  });

  it('uses defaultValue when provided', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'image', type: 'text', defaultValue: 'ubuntu:22.04' }),
        makeField({ name: 'cpu', type: 'number', defaultValue: 4 }),
        makeField({ name: 'ssh', type: 'boolean', defaultValue: true }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values.image).toBe('ubuntu:22.04');
    expect(result.current.values.cpu).toBe(4);
    expect(result.current.values.ssh).toBe(true);
  });

  it('skips internal fields', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'name', type: 'text' }),
        makeField({ name: 'token', type: 'text', internal: true }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values).toHaveProperty('name');
    expect(result.current.values).not.toHaveProperty('token');
  });

  it('returns empty state for null job', () => {
    const { result } = renderHook(() => useJobForm(null));
    expect(result.current.values).toEqual({});
    expect(result.current.errors).toEqual({});
  });
});

// ── Defaults from real blueprints ──

describe('useJobForm with real blueprint jobs', () => {
  it('initializes sandbox_create with correct defaults', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 0)!;
    const { result } = renderHook(() => useJobForm(job));
    const v = result.current.values;
    expect(v.name).toBe('');
    expect(v.image).toBe('ubuntu:22.04');
    expect(v.stack).toBe('default');
    expect(v.cpuCores).toBe(2);
    expect(v.memoryMb).toBe(2048);
    expect(v.diskGb).toBe(10);
    expect(v.sshEnabled).toBe(false);
    expect(v.webTerminalEnabled).toBe(true);
    expect(v.maxLifetimeSeconds).toBe(86400);
    expect(v.idleTimeoutSeconds).toBe(3600);
    expect(v.teeRequired).toBe(false);
    expect(v.teeType).toBe('0');
    // sidecarToken should NOT be present (internal)
    expect(v).not.toHaveProperty('sidecarToken');
  });

  it('initializes instance_provision with sidecarToken excluded', () => {
    const job = getJobById('ai-agent-instance-blueprint', INSTANCE_JOB_IDS.PROVISION)!;
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values).not.toHaveProperty('sidecarToken');
    expect(result.current.values.name).toBe('');
  });

  it('initializes TEE instance_provision with teeRequired=true', () => {
    const job = getJobById('ai-agent-tee-instance-blueprint', INSTANCE_JOB_IDS.PROVISION)!;
    const { result } = renderHook(() => useJobForm(job));
    expect(result.current.values.teeRequired).toBe(true);
    expect(result.current.values.teeType).toBe('1');
  });
});

// ── onChange ──

describe('useJobForm onChange', () => {
  it('updates a single field', () => {
    const job = makeJob({
      fields: [makeField({ name: 'name', type: 'text' })],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('name', 'my-sandbox'));
    expect(result.current.values.name).toBe('my-sandbox');
  });

  it('clears error on field change', () => {
    const job = makeJob({
      fields: [makeField({ name: 'name', type: 'text', required: true })],
    });
    const { result } = renderHook(() => useJobForm(job));

    // Trigger validation to create error
    act(() => { result.current.validate(); });
    expect(result.current.errors.name).toBeDefined();

    // Change field — error should clear
    act(() => result.current.onChange('name', 'fixed'));
    expect(result.current.errors.name).toBeUndefined();
  });

  it('preserves other fields on change', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'a', type: 'text', defaultValue: 'alpha' }),
        makeField({ name: 'b', type: 'text', defaultValue: 'beta' }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('a', 'changed'));
    expect(result.current.values.a).toBe('changed');
    expect(result.current.values.b).toBe('beta');
  });
});

// ── Validation ──

describe('useJobForm validate', () => {
  it('passes when all required fields filled', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'name', type: 'text', required: true }),
        makeField({ name: 'optional', type: 'text' }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('name', 'test'));
    let valid = false;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(true);
    expect(result.current.errors).toEqual({});
  });

  it('fails when required fields empty', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'name', type: 'text', required: true }),
        makeField({ name: 'image', type: 'text', required: true }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));
    let valid = true;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(false);
    expect(result.current.errors.name).toBeDefined();
    expect(result.current.errors.image).toBeDefined();
  });

  it('skips validation for internal required fields', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'name', type: 'text', required: true }),
        makeField({ name: 'token', type: 'text', required: true, internal: true }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('name', 'test'));
    let valid = false;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(true);
  });

  it('returns false for null job', () => {
    const { result } = renderHook(() => useJobForm(null));
    let valid = true;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(false);
  });

  it('error messages include field label', () => {
    const job = makeJob({
      fields: [makeField({ name: 'name', type: 'text', label: 'Sandbox Name', required: true })],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => { result.current.validate(); });
    expect(result.current.errors.name).toContain('Sandbox Name');
  });
});

// ── Numeric bounds validation ──

describe('useJobForm numeric bounds', () => {
  it('fails when number is below min', () => {
    const job = makeJob({
      fields: [makeField({ name: 'cpu', type: 'number', label: 'CPU Cores', min: 1, defaultValue: 2 })],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('cpu', -1));
    let valid = true;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(false);
    expect(result.current.errors.cpu).toContain('at least 1');
  });

  it('fails when number exceeds max', () => {
    const job = makeJob({
      fields: [makeField({ name: 'count', type: 'number', label: 'Count', max: 100, defaultValue: 3 })],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('count', 200));
    let valid = true;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(false);
    expect(result.current.errors.count).toContain('at most 100');
  });

  it('passes when number is within min/max range', () => {
    const job = makeJob({
      fields: [makeField({ name: 'timeout', type: 'number', label: 'Timeout', min: 0, max: 600000, defaultValue: 30000 })],
    });
    const { result } = renderHook(() => useJobForm(job));
    let valid = false;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(true);
    expect(result.current.errors).toEqual({});
  });

  it('allows zero when min is 0', () => {
    const job = makeJob({
      fields: [makeField({ name: 'timeout', type: 'number', label: 'Timeout', min: 0, defaultValue: 30000 })],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('timeout', 0));
    let valid = false;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(true);
  });

  it('validates real sandbox cpu field rejects 0', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 0)!;
    const { result } = renderHook(() => useJobForm(job));
    act(() => result.current.onChange('cpuCores', 0));
    let valid = true;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(false);
    expect(result.current.errors.cpuCores).toContain('at least 1');
  });

  it('validates real sandbox maxLifetimeSeconds allows 0', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.SANDBOX_CREATE)!;
    const { result } = renderHook(() => useJobForm(job));
    act(() => {
      result.current.onChange('name', 'test'); // fill required field
      result.current.onChange('image', 'ubuntu:22.04'); // fill required field
      result.current.onChange('maxLifetimeSeconds', 0);
    });
    let valid = false;
    act(() => { valid = result.current.validate(); });
    expect(valid).toBe(true);
  });
});

// ── Reset ──

describe('useJobForm reset', () => {
  it('restores all fields to defaults', () => {
    const job = makeJob({
      fields: [
        makeField({ name: 'name', type: 'text', defaultValue: 'default' }),
        makeField({ name: 'count', type: 'number', defaultValue: 5 }),
      ],
    });
    const { result } = renderHook(() => useJobForm(job));

    act(() => {
      result.current.onChange('name', 'changed');
      result.current.onChange('count', 99);
    });
    expect(result.current.values.name).toBe('changed');

    act(() => result.current.reset());
    expect(result.current.values.name).toBe('default');
    expect(result.current.values.count).toBe(5);
  });

  it('clears errors on reset', () => {
    const job = makeJob({
      fields: [makeField({ name: 'name', type: 'text', required: true })],
    });
    const { result } = renderHook(() => useJobForm(job));
    act(() => { result.current.validate(); });
    expect(Object.keys(result.current.errors).length).toBeGreaterThan(0);

    act(() => result.current.reset());
    expect(result.current.errors).toEqual({});
  });
});
