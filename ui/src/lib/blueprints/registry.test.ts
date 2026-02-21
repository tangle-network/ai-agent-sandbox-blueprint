import { describe, it, expect } from 'vitest';
import './index'; // auto-register all blueprints
import { getAllBlueprints, getBlueprint, getBlueprintJobs, getJobById } from './registry';
import { JOB_IDS } from '~/lib/types/sandbox';
import { INSTANCE_JOB_IDS } from '~/lib/types/instance';

// ── Registry API ──

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

  it('retrieves sandbox blueprint with 5 on-chain jobs', () => {
    const bp = getBlueprint('ai-agent-sandbox-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent Sandbox');
    expect(bp!.jobs.length).toBe(5);
  });

  it('retrieves instance blueprint with 2 on-chain jobs', () => {
    const bp = getBlueprint('ai-agent-instance-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent Instance');
    expect(bp!.jobs.length).toBe(2);
  });

  it('retrieves TEE instance blueprint with 2 on-chain jobs', () => {
    const bp = getBlueprint('ai-agent-tee-instance-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent TEE Instance');
    expect(bp!.jobs.length).toBe(2);
  });

  it('filters jobs by category', () => {
    const lifecycle = getBlueprintJobs('ai-agent-sandbox-blueprint', 'lifecycle');
    expect(lifecycle.length).toBeGreaterThan(0);
    expect(lifecycle.every((j) => j.category === 'lifecycle')).toBe(true);
  });

  it('finds job by id', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.SANDBOX_CREATE);
    expect(job).toBeDefined();
    expect(job!.name).toBe('sandbox_create');
  });

  it('returns undefined for unknown blueprint', () => {
    expect(getBlueprint('nonexistent')).toBeUndefined();
  });

  it('returns empty array for unknown blueprint jobs', () => {
    expect(getBlueprintJobs('nonexistent')).toEqual([]);
  });

  it('returns undefined for unknown job id', () => {
    expect(getJobById('ai-agent-sandbox-blueprint', 999)).toBeUndefined();
  });
});

// ── ABI Metadata Correctness ──

describe('Blueprint ABI Metadata', () => {
  it('sandbox create has abiType on all fields', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.SANDBOX_CREATE)!;
    const fieldsWithAbi = job.fields.filter((f) => f.abiType);
    // Every field in sandbox_create must have an abiType for encoding
    expect(fieldsWithAbi.length).toBe(job.fields.length);
  });

  it('sandbox delete has sandbox_id context param', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.SANDBOX_DELETE)!;
    expect(job.contextParams).toBeDefined();
    expect(job.contextParams![0].abiName).toBe('sandbox_id');
  });

  it('instance provision has sidecar_token as internal field', () => {
    const job = getJobById('ai-agent-instance-blueprint', INSTANCE_JOB_IDS.PROVISION)!;
    const internal = job.fields.filter((f) => f.internal);
    expect(internal.length).toBeGreaterThan(0);
    expect(internal.some((f) => f.name === 'sidecarToken')).toBe(true);
  });

  it('TEE instance provision defaults teeRequired to true', () => {
    const job = getJobById('ai-agent-tee-instance-blueprint', INSTANCE_JOB_IDS.PROVISION)!;
    const teeField = job.fields.find((f) => f.name === 'teeRequired');
    expect(teeField).toBeDefined();
    expect(teeField!.defaultValue).toBe(true);
  });

  it('TEE instance provision defaults teeType to TDX (1)', () => {
    const job = getJobById('ai-agent-tee-instance-blueprint', INSTANCE_JOB_IDS.PROVISION)!;
    const teeType = job.fields.find((f) => f.name === 'teeType');
    expect(teeType!.defaultValue).toBe('1');
  });

  it('every job with requiresSandbox has either contextParams or is instance-scoped', () => {
    const sandboxBp = getBlueprint('ai-agent-sandbox-blueprint')!;
    for (const job of sandboxBp.jobs) {
      if (job.requiresSandbox) {
        expect(
          job.contextParams?.length,
          `Sandbox job ${job.name} (${job.id}) should have context params`,
        ).toBeGreaterThan(0);
      }
    }
  });

  it('all abiParam values use snake_case', () => {
    for (const bp of getAllBlueprints()) {
      for (const job of bp.jobs) {
        for (const field of job.fields) {
          if (field.abiParam) {
            expect(
              field.abiParam,
              `${bp.id}/${job.name}/${field.name}: abiParam should be snake_case`,
            ).toMatch(/^[a-z][a-z0-9_]*$/);
          }
        }
      }
    }
  });
});

// ── Category Coverage ──

describe('Blueprint Categories', () => {
  it('sandbox blueprint covers lifecycle and workflow categories', () => {
    const bp = getBlueprint('ai-agent-sandbox-blueprint')!;
    const cats = new Set(bp.jobs.map((j) => j.category));
    expect(cats).toContain('lifecycle');
    expect(cats).toContain('workflow');
  });

  it('instance blueprint covers lifecycle category', () => {
    const bp = getBlueprint('ai-agent-instance-blueprint')!;
    const cats = new Set(bp.jobs.map((j) => j.category));
    expect(cats).toContain('lifecycle');
  });

  it('every registered category has at least one job', () => {
    for (const bp of getAllBlueprints()) {
      for (const cat of bp.categories) {
        const jobs = bp.jobs.filter((j) => j.category === cat.key);
        expect(
          jobs.length,
          `${bp.id} category '${cat.key}' should have jobs`,
        ).toBeGreaterThan(0);
      }
    }
  });
});
