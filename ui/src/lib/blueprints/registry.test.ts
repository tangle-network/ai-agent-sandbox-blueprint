import { describe, it, expect, beforeAll } from 'vitest';
import './index'; // auto-register all blueprints
import { getAllBlueprints, getBlueprint, getBlueprintJobs, getJobById } from './registry';

beforeAll(() => {
  // Blueprints are auto-registered via import above
});

describe('Blueprint Registry', () => {
  it('registers all 3 blueprints', () => {
    const all = getAllBlueprints();
    expect(all.length).toBe(3);
    expect(all.map((b) => b.id).sort()).toEqual([
      'ai-agent-instance-blueprint',
      'ai-agent-sandbox-blueprint',
      'ai-agent-tee-instance-blueprint',
    ]);
  });

  it('retrieves sandbox blueprint by id', () => {
    const bp = getBlueprint('ai-agent-sandbox-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent Sandbox');
    expect(bp!.jobs.length).toBe(17);
  });

  it('retrieves instance blueprint by id', () => {
    const bp = getBlueprint('ai-agent-instance-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent Instance');
    expect(bp!.jobs.length).toBe(8);
  });

  it('retrieves TEE instance blueprint by id', () => {
    const bp = getBlueprint('ai-agent-tee-instance-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent TEE Instance');
    expect(bp!.jobs.length).toBe(8);
  });

  it('filters jobs by category', () => {
    const lifecycle = getBlueprintJobs('ai-agent-sandbox-blueprint', 'lifecycle');
    expect(lifecycle.length).toBeGreaterThan(0);
    expect(lifecycle.every((j) => j.category === 'lifecycle')).toBe(true);
  });

  it('finds job by id', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 0);
    expect(job).toBeDefined();
    expect(job!.name).toBe('sandbox_create');
  });

  it('returns undefined for unknown blueprint', () => {
    expect(getBlueprint('nonexistent')).toBeUndefined();
  });

  it('returns empty array for unknown blueprint jobs', () => {
    expect(getBlueprintJobs('nonexistent')).toEqual([]);
  });
});

describe('Blueprint ABI metadata', () => {
  it('sandbox create has abiType on all fields', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 0)!;
    const fieldsWithAbi = job.fields.filter((f) => f.abiType);
    // Should have all fields with ABI types
    expect(fieldsWithAbi.length).toBeGreaterThanOrEqual(14);
  });

  it('sandbox exec has sidecar_url context param', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 10)!;
    expect(job.contextParams).toBeDefined();
    expect(job.contextParams![0].abiName).toBe('sidecar_url');
    expect(job.contextParams![0].abiType).toBe('string');
  });

  it('instance exec has no context params', () => {
    const job = getJobById('ai-agent-instance-blueprint', 1)!;
    expect(job.contextParams).toBeUndefined();
  });

  it('instance provision has sidecar_token as internal field', () => {
    const job = getJobById('ai-agent-instance-blueprint', 0)!;
    const internal = job.fields.filter((f) => f.internal);
    expect(internal.length).toBeGreaterThan(0);
    expect(internal.some((f) => f.name === 'sidecarToken')).toBe(true);
  });

  it('TEE instance provision defaults teeRequired to true', () => {
    const job = getJobById('ai-agent-tee-instance-blueprint', 0)!;
    const teeField = job.fields.find((f) => f.name === 'teeRequired');
    expect(teeField).toBeDefined();
    expect(teeField!.defaultValue).toBe(true);
  });
});
