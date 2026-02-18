import { describe, it, expect } from 'vitest';
import './index'; // auto-register all blueprints
import { getAllBlueprints, getBlueprint, getBlueprintJobs, getJobById } from './registry';

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

  it('retrieves sandbox blueprint with 17 jobs', () => {
    const bp = getBlueprint('ai-agent-sandbox-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent Sandbox');
    expect(bp!.jobs.length).toBe(17);
  });

  it('retrieves instance blueprint with 8 jobs', () => {
    const bp = getBlueprint('ai-agent-instance-blueprint');
    expect(bp).toBeDefined();
    expect(bp!.name).toBe('AI Agent Instance');
    expect(bp!.jobs.length).toBe(8);
  });

  it('retrieves TEE instance blueprint with 8 jobs', () => {
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

  it('returns undefined for unknown job id', () => {
    expect(getJobById('ai-agent-sandbox-blueprint', 999)).toBeUndefined();
  });
});

// ── ABI Metadata Correctness ──

describe('Blueprint ABI Metadata', () => {
  it('sandbox create has abiType on all 16 fields', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 0)!;
    const fieldsWithAbi = job.fields.filter((f) => f.abiType);
    expect(fieldsWithAbi.length).toBe(16);
  });

  it('sandbox exec has sidecar_url context param', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 10)!;
    expect(job.contextParams).toBeDefined();
    expect(job.contextParams![0].abiName).toBe('sidecar_url');
    expect(job.contextParams![0].abiType).toBe('string');
  });

  it('sandbox stop/resume/delete have sandbox_id context param', () => {
    for (const id of [1, 2, 3]) {
      const job = getJobById('ai-agent-sandbox-blueprint', id)!;
      expect(job.contextParams, `job ${id} should have contextParams`).toBeDefined();
      expect(job.contextParams![0].abiName).toBe('sandbox_id');
    }
  });

  it('instance exec has no context params (instance-scoped)', () => {
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

  it('TEE instance provision defaults teeType to TDX (1)', () => {
    const job = getJobById('ai-agent-tee-instance-blueprint', 0)!;
    const teeType = job.fields.find((f) => f.name === 'teeType');
    expect(teeType!.defaultValue).toBe('1');
  });

  it('batch_create has customEncoder for nested struct', () => {
    const job = getJobById('ai-agent-sandbox-blueprint', 20)!;
    expect(job.customEncoder).toBeDefined();
    expect(typeof job.customEncoder).toBe('function');
  });

  it('every job with requiresSandbox has either contextParams or is instance-scoped', () => {
    const sandboxBp = getBlueprint('ai-agent-sandbox-blueprint')!;
    for (const job of sandboxBp.jobs) {
      if (job.requiresSandbox && job.id !== 20) { // batch_create is special
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
  it('sandbox blueprint covers all 5 categories', () => {
    const bp = getBlueprint('ai-agent-sandbox-blueprint')!;
    const cats = new Set(bp.jobs.map((j) => j.category));
    expect(cats).toContain('lifecycle');
    expect(cats).toContain('execution');
    expect(cats).toContain('batch');
    expect(cats).toContain('workflow');
    expect(cats).toContain('ssh');
  });

  it('instance blueprint covers lifecycle, execution, ssh, management', () => {
    const bp = getBlueprint('ai-agent-instance-blueprint')!;
    const cats = new Set(bp.jobs.map((j) => j.category));
    expect(cats).toContain('lifecycle');
    expect(cats).toContain('execution');
    expect(cats).toContain('ssh');
    expect(cats).toContain('management');
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
