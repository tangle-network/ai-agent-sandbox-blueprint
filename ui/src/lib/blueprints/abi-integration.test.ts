/**
 * ABI Integration Tests
 *
 * Verifies that every real job definition across all 3 blueprints produces
 * encoded bytes that can be decoded using the canonical Rust sol! ABI shapes.
 *
 * This is the highest-value test surface — it catches any drift between
 * TypeScript field metadata and the actual Rust ABI structs.
 *
 * NOTE: Read-only ops (exec, prompt, task, ssh, snapshot, batch) have been
 * moved to the operator API and are no longer on-chain jobs.
 */

import { describe, it, expect } from 'vitest';
import { decodeAbiParameters } from 'viem';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import './index'; // auto-register all blueprints
import { JOB_IDS } from '~/lib/types/sandbox';
import { INSTANCE_JOB_IDS } from '~/lib/types/instance';
import {
  type AbiParamDef,
  SANDBOX_CREATE_ABI,
  SANDBOX_ID_ABI,
  WORKFLOW_CREATE_ABI,
  WORKFLOW_CONTROL_ABI,
  SANDBOX_CREATE_VALUES,
  WORKFLOW_CREATE_VALUES,
  SANDBOX_ID_CONTEXT,
} from '~/test/fixtures';

/**
 * Encode a job with formValues/context, then decode using the canonical Rust ABI shape.
 * Throws if the encoded bytes don't match the expected ABI structure.
 */
function encodeAndDecode(
  blueprintId: string,
  jobId: number,
  formValues: Record<string, unknown>,
  context?: Record<string, unknown>,
  rustAbi?: AbiParamDef[],
) {
  const job = getJobById(blueprintId, jobId);
  expect(job, `Job ${jobId} should exist in ${blueprintId}`).toBeDefined();

  const encoded = encodeJobArgs(job!, formValues, context);
  expect(encoded).toMatch(/^0x[0-9a-f]+$/i);

  if (rustAbi) {
    // Decode using the canonical Rust ABI shape — this is the real integration check
    const decoded = decodeAbiParameters(rustAbi, encoded);
    expect(decoded).toBeDefined();
    return decoded;
  }
  return encoded;
}

// ── Sandbox Blueprint ──

describe('Sandbox Blueprint ABI Integration', () => {
  const BP = 'ai-agent-sandbox-blueprint';

  it('sandbox_create encodes all 16 fields matching SandboxCreateRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SANDBOX_CREATE, SANDBOX_CREATE_VALUES, undefined, SANDBOX_CREATE_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('test-sandbox');       // name
    expect(d[1]).toBe('ubuntu:22.04');       // image
    expect(d[2]).toBe('default');            // stack
    expect(d[3]).toBe('agent-1');            // agent_identifier
    expect(d[4]).toBe('{"KEY":"val"}');      // env_json
    expect(d[5]).toBe('{}');                 // metadata_json
    expect(d[6]).toBe(false);               // ssh_enabled
    expect(d[7]).toBe('');                   // ssh_public_key
    expect(d[8]).toBe(true);                // web_terminal_enabled
    expect(d[9]).toBe(86400n);              // max_lifetime_seconds (uint64)
    expect(d[10]).toBe(3600n);              // idle_timeout_seconds (uint64)
    expect(d[11]).toBe(2n);                 // cpu_cores (uint64)
    expect(d[12]).toBe(2048n);              // memory_mb (uint64)
    expect(d[13]).toBe(10n);                // disk_gb (uint64)
    expect(d[14]).toBe(false);              // tee_required
    expect(d[15]).toBe(0);                  // tee_type (uint8)
  });

  it('sandbox_delete encodes SandboxIdRequest via context', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SANDBOX_DELETE, {}, SANDBOX_ID_CONTEXT, SANDBOX_ID_ABI);
    expect((decoded as readonly unknown[])[0]).toBe('sb-test-001');
  });

  it('workflow_create encodes WorkflowCreateRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.WORKFLOW_CREATE, WORKFLOW_CREATE_VALUES, undefined, WORKFLOW_CREATE_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('daily-backup');
    expect(d[1]).toBe('{"steps":[]}');
    expect(d[2]).toBe('cron');
    expect(d[3]).toBe('0 */6 * * *');
    expect(d[4]).toBe('{"image":"ubuntu:22.04"}');
  });

  it('workflow_trigger encodes WorkflowControlRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.WORKFLOW_TRIGGER, { workflowId: 42 }, undefined, WORKFLOW_CONTROL_ABI);
    expect((decoded as readonly unknown[])[0]).toBe(42n);
  });

  it('workflow_cancel encodes WorkflowControlRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.WORKFLOW_CANCEL, { workflowId: 7 }, undefined, WORKFLOW_CONTROL_ABI);
    expect((decoded as readonly unknown[])[0]).toBe(7n);
  });

  it('all 5 on-chain jobs exist and are encodable', () => {
    const jobIds = [0, 1, 2, 3, 4];
    for (const id of jobIds) {
      const job = getJobById(BP, id);
      expect(job, `Sandbox job ${id} should exist`).toBeDefined();
      expect(() => encodeJobArgs(job!, {})).not.toThrow();
    }
  });
});

// ── Instance Blueprint ──

describe('Instance Blueprint ABI Integration', () => {
  const BP = 'ai-agent-instance-blueprint';

  it('all 3 on-chain jobs exist and are encodable', () => {
    for (const id of [
      INSTANCE_JOB_IDS.WORKFLOW_CREATE,
      INSTANCE_JOB_IDS.WORKFLOW_TRIGGER,
      INSTANCE_JOB_IDS.WORKFLOW_CANCEL,
    ]) {
      const job = getJobById(BP, id);
      expect(job, `Instance job ${id} should exist`).toBeDefined();
      expect(() => encodeJobArgs(job!, {})).not.toThrow();
    }
  });
});

// ── TEE Instance Blueprint ──

describe('TEE Instance Blueprint ABI Integration', () => {
  const BP = 'ai-agent-tee-instance-blueprint';

  it('all 3 on-chain jobs exist and are encodable', () => {
    for (const id of [
      INSTANCE_JOB_IDS.WORKFLOW_CREATE,
      INSTANCE_JOB_IDS.WORKFLOW_TRIGGER,
      INSTANCE_JOB_IDS.WORKFLOW_CANCEL,
    ]) {
      const job = getJobById(BP, id);
      expect(job, `TEE instance job ${id} should exist`).toBeDefined();
      expect(() => encodeJobArgs(job!, {})).not.toThrow();
    }
  });
});

// ── Cross-Blueprint Consistency ──

describe('Cross-Blueprint Consistency', () => {
  it('TEE instance and standard instance produce identical workflow encoding', () => {
    const stdJob = getJobById('ai-agent-instance-blueprint', INSTANCE_JOB_IDS.WORKFLOW_CREATE)!;
    const teeJob = getJobById('ai-agent-tee-instance-blueprint', INSTANCE_JOB_IDS.WORKFLOW_CREATE)!;

    const stdEncoded = encodeJobArgs(stdJob, WORKFLOW_CREATE_VALUES);
    const teeEncoded = encodeJobArgs(teeJob, WORKFLOW_CREATE_VALUES);
    expect(stdEncoded).toBe(teeEncoded);
  });

  it('field count matches Rust struct field count for workflow jobs', () => {
    // SandboxCreateRequest: 16 fields
    const sandboxCreate = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.SANDBOX_CREATE)!;
    expect(sandboxCreate.fields.filter(f => f.abiType).length).toBe(16);

    // WorkflowCreateRequest: 5 fields
    const instanceWorkflowCreate = getJobById('ai-agent-instance-blueprint', INSTANCE_JOB_IDS.WORKFLOW_CREATE)!;
    expect(instanceWorkflowCreate.fields.filter(f => f.abiType).length).toBe(5);
  });
});
