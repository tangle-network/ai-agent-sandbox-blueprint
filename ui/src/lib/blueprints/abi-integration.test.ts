/**
 * ABI Integration Tests
 *
 * Verifies that every real job definition across all 3 blueprints produces
 * encoded bytes that can be decoded using the canonical Rust sol! ABI shapes.
 *
 * This is the highest-value test surface — it catches any drift between
 * TypeScript field metadata and the actual Rust ABI structs.
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { decodeAbiParameters } from 'viem';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getJobById } from './registry';
import './index'; // auto-register all blueprints
import { JOB_IDS } from '~/lib/types/sandbox';
import { INSTANCE_JOB_IDS } from '~/lib/types/instance';
import {
  type AbiParamDef,
  SANDBOX_CREATE_ABI,
  SANDBOX_ID_ABI,
  SANDBOX_SNAPSHOT_ABI,
  SANDBOX_EXEC_ABI,
  SANDBOX_PROMPT_ABI,
  SANDBOX_TASK_ABI,
  BATCH_TASK_ABI,
  BATCH_EXEC_ABI,
  BATCH_COLLECT_ABI,
  WORKFLOW_CREATE_ABI,
  WORKFLOW_CONTROL_ABI,
  SSH_REQUEST_ABI,
  INSTANCE_PROVISION_ABI,
  INSTANCE_EXEC_ABI,
  INSTANCE_PROMPT_ABI,
  INSTANCE_TASK_ABI,
  INSTANCE_SSH_ABI,
  INSTANCE_SNAPSHOT_ABI,
  JSON_REQUEST_ABI,
  SANDBOX_CREATE_VALUES,
  INSTANCE_PROVISION_VALUES,
  EXEC_VALUES,
  PROMPT_VALUES,
  TASK_VALUES,
  SSH_VALUES,
  SNAPSHOT_VALUES,
  WORKFLOW_CREATE_VALUES,
  SIDECAR_CONTEXT,
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

  it('sandbox_stop encodes SandboxIdRequest via context', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SANDBOX_STOP, {}, SANDBOX_ID_CONTEXT, SANDBOX_ID_ABI);
    expect((decoded as readonly unknown[])[0]).toBe('sb-test-001');
  });

  it('sandbox_resume encodes SandboxIdRequest via context', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SANDBOX_RESUME, {}, SANDBOX_ID_CONTEXT, SANDBOX_ID_ABI);
    expect((decoded as readonly unknown[])[0]).toBe('sb-test-001');
  });

  it('sandbox_delete encodes SandboxIdRequest via context', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SANDBOX_DELETE, {}, SANDBOX_ID_CONTEXT, SANDBOX_ID_ABI);
    expect((decoded as readonly unknown[])[0]).toBe('sb-test-001');
  });

  it('sandbox_snapshot encodes SandboxSnapshotRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SANDBOX_SNAPSHOT, SNAPSHOT_VALUES, SIDECAR_CONTEXT, SANDBOX_SNAPSHOT_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('http://localhost:8080');  // sidecar_url
    expect(d[1]).toBe('s3://bucket/snapshot-001'); // destination
    expect(d[2]).toBe(true);                     // include_workspace
    expect(d[3]).toBe(true);                     // include_state
  });

  it('exec encodes SandboxExecRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.EXEC, EXEC_VALUES, SIDECAR_CONTEXT, SANDBOX_EXEC_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('http://localhost:8080');      // sidecar_url
    expect(d[1]).toBe('ls -la /workspace');          // command
    expect(d[2]).toBe('/workspace');                 // cwd
    expect(d[3]).toBe('{}');                         // env_json
    expect(d[4]).toBe(30000n);                       // timeout_ms
  });

  it('prompt encodes SandboxPromptRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.PROMPT, PROMPT_VALUES, SIDECAR_CONTEXT, SANDBOX_PROMPT_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('http://localhost:8080');
    expect(d[1]).toBe('What files are in the workspace?');
    expect(d[2]).toBe('sess-123');
    expect(d[3]).toBe('claude-3');
    expect(d[4]).toBe('{}');
    expect(d[5]).toBe(60000n);
  });

  it('task encodes SandboxTaskRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.TASK, TASK_VALUES, SIDECAR_CONTEXT, SANDBOX_TASK_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('http://localhost:8080');
    expect(d[1]).toBe('Build a REST API');
    expect(d[2]).toBe('sess-456');
    expect(d[3]).toBe(10n);  // max_turns
    expect(d[4]).toBe('claude-3');
    expect(d[5]).toBe('{}');
    expect(d[6]).toBe(300000n);
  });

  it('batch_create encodes BatchCreateRequest with nested struct', () => {
    // batch_create uses customEncoder — still must match Rust ABI
    const job = getJobById(BP, JOB_IDS.BATCH_CREATE)!;
    const encoded = encodeJobArgs(job, {
      count: 3,
      configJson: JSON.stringify({
        name: 'batch', image: 'ubuntu:22.04', stack: 'default',
        agent_identifier: '', env_json: '{}', metadata_json: '{}',
        ssh_enabled: false, ssh_public_key: '', web_terminal_enabled: true,
        max_lifetime_seconds: 86400, idle_timeout_seconds: 3600,
        cpu_cores: 2, memory_mb: 2048, disk_gb: 10,
        tee_required: false, tee_type: 0,
      }),
      operators: '0x1234567890abcdef1234567890abcdef12345678',
      distribution: 'round_robin',
    });
    // Decode using canonical BatchCreateRequest ABI
    const decoded = decodeAbiParameters(
      [
        { name: 'count', type: 'uint32' },
        {
          name: 'template_request', type: 'tuple',
          components: [
            { name: 'name', type: 'string' }, { name: 'image', type: 'string' },
            { name: 'stack', type: 'string' }, { name: 'agent_identifier', type: 'string' },
            { name: 'env_json', type: 'string' }, { name: 'metadata_json', type: 'string' },
            { name: 'ssh_enabled', type: 'bool' }, { name: 'ssh_public_key', type: 'string' },
            { name: 'web_terminal_enabled', type: 'bool' },
            { name: 'max_lifetime_seconds', type: 'uint64' }, { name: 'idle_timeout_seconds', type: 'uint64' },
            { name: 'cpu_cores', type: 'uint64' }, { name: 'memory_mb', type: 'uint64' }, { name: 'disk_gb', type: 'uint64' },
            { name: 'tee_required', type: 'bool' }, { name: 'tee_type', type: 'uint8' },
          ],
        },
        { name: 'operators', type: 'address[]' },
        { name: 'distribution', type: 'string' },
      ],
      encoded,
    );
    expect(decoded[0]).toBe(3); // count (uint32)
    // viem decodes tuples as objects with named + positional keys
    const template = decoded[1] as unknown as Record<string, unknown>;
    expect(template.name ?? template[0]).toBe('batch');
    expect(template.image ?? template[1]).toBe('ubuntu:22.04');
    expect(template.cpu_cores ?? template[11]).toBe(2n); // uint64
    expect((decoded[2] as readonly unknown[]).length).toBe(1); // 1 operator
    expect(decoded[3]).toBe('round_robin');
  });

  it('batch_task encodes BatchTaskRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.BATCH_TASK, {
      sidecarUrls: 'http://s1:3000\nhttp://s2:3000',
      prompt: 'Analyze logs',
      sessionId: '',
      maxTurns: 5,
      model: '',
      contextJson: '{}',
      timeoutMs: 300000,
      parallel: true,
      aggregation: 'collect',
    }, undefined, BATCH_TASK_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toEqual(['http://s1:3000', 'http://s2:3000']); // sidecar_urls[]
    expect(d[1]).toBe('Analyze logs');
    expect(d[3]).toBe(5n);   // max_turns
    expect(d[7]).toBe(true); // parallel
    expect(d[8]).toBe('collect');
  });

  it('batch_exec encodes BatchExecRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.BATCH_EXEC, {
      sidecarUrls: 'http://s1:3000',
      command: 'uptime',
      cwd: '',
      envJson: '{}',
      timeoutMs: 10000,
      parallel: false,
    }, undefined, BATCH_EXEC_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toEqual(['http://s1:3000']);
    expect(d[1]).toBe('uptime');
    expect(d[5]).toBe(false); // parallel
  });

  it('batch_collect encodes BatchCollectRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.BATCH_COLLECT, { batchId: 'batch-001' }, undefined, BATCH_COLLECT_ABI);
    expect((decoded as readonly unknown[])[0]).toBe('batch-001');
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

  it('ssh_provision encodes SshProvisionRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SSH_PROVISION, SSH_VALUES, SIDECAR_CONTEXT, SSH_REQUEST_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('http://localhost:8080');
    expect(d[1]).toBe('agent');
    expect(d[2]).toBe('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest');
  });

  it('ssh_revoke encodes SshRevokeRequest', () => {
    const decoded = encodeAndDecode(BP, JOB_IDS.SSH_REVOKE, SSH_VALUES, SIDECAR_CONTEXT, SSH_REQUEST_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('http://localhost:8080');
    expect(d[1]).toBe('agent');
    expect(d[2]).toBe('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest');
  });
});

// ── Instance Blueprint ──

describe('Instance Blueprint ABI Integration', () => {
  const BP = 'ai-agent-instance-blueprint';

  it('provision encodes ProvisionRequest with sidecar_token', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.PROVISION, INSTANCE_PROVISION_VALUES, undefined, INSTANCE_PROVISION_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('test-instance');       // name
    expect(d[1]).toBe('ubuntu:22.04');       // image
    expect(d[14]).toBe('');                  // sidecar_token (internal, default empty)
    expect(d[15]).toBe(false);              // tee_required
    expect(d[16]).toBe(0);                  // tee_type
  });

  it('exec encodes InstanceExecRequest (no context params)', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.EXEC, EXEC_VALUES, undefined, INSTANCE_EXEC_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('ls -la /workspace');
    expect(d[1]).toBe('/workspace');
    expect(d[2]).toBe('{}');
    expect(d[3]).toBe(30000n);
  });

  it('prompt encodes InstancePromptRequest (no context params)', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.PROMPT, PROMPT_VALUES, undefined, INSTANCE_PROMPT_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('What files are in the workspace?');
    expect(d[1]).toBe('sess-123');
    expect(d[2]).toBe('claude-3');
    expect(d[3]).toBe('{}');
    expect(d[4]).toBe(60000n);
  });

  it('task encodes InstanceTaskRequest (no context params)', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.TASK, TASK_VALUES, undefined, INSTANCE_TASK_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('Build a REST API');
    expect(d[1]).toBe('sess-456');
    expect(d[2]).toBe(10n);
    expect(d[3]).toBe('claude-3');
    expect(d[4]).toBe('{}');
    expect(d[5]).toBe(300000n);
  });

  it('ssh_provision encodes InstanceSshProvisionRequest (no sidecar_url)', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.SSH_PROVISION, SSH_VALUES, undefined, INSTANCE_SSH_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('agent');
    expect(d[1]).toBe('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest');
  });

  it('ssh_revoke encodes InstanceSshRevokeRequest (no sidecar_url)', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.SSH_REVOKE, SSH_VALUES, undefined, INSTANCE_SSH_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('agent');
    expect(d[1]).toBe('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest');
  });

  it('snapshot encodes InstanceSnapshotRequest', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.SNAPSHOT, SNAPSHOT_VALUES, undefined, INSTANCE_SNAPSHOT_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('s3://bucket/snapshot-001');
    expect(d[1]).toBe(true);
    expect(d[2]).toBe(true);
  });

  it('deprovision encodes JsonRequest', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.DEPROVISION, { json: '{}' }, undefined, JSON_REQUEST_ABI);
    expect((decoded as readonly unknown[])[0]).toBe('{}');
  });
});

// ── TEE Instance Blueprint ──

describe('TEE Instance Blueprint ABI Integration', () => {
  const BP = 'ai-agent-tee-instance-blueprint';

  it('provision encodes ProvisionRequest with TEE defaults', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.PROVISION, {
      ...INSTANCE_PROVISION_VALUES,
      teeRequired: true,
      teeType: '1',
    }, undefined, INSTANCE_PROVISION_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('test-instance');
    expect(d[15]).toBe(true);  // tee_required
    expect(d[16]).toBe(1);     // tee_type = TDX
  });

  it('exec encodes same InstanceExecRequest ABI as standard instance', () => {
    const decoded = encodeAndDecode(BP, INSTANCE_JOB_IDS.EXEC, EXEC_VALUES, undefined, INSTANCE_EXEC_ABI);
    const d = decoded as readonly unknown[];
    expect(d[0]).toBe('ls -la /workspace');
    expect(d[3]).toBe(30000n);
  });

  it('all 8 jobs exist and are encodable', () => {
    const jobIds = [0, 1, 2, 3, 4, 5, 6, 7];
    for (const id of jobIds) {
      const job = getJobById(BP, id);
      expect(job, `TEE instance job ${id} should exist`).toBeDefined();
      // Just verify encoding doesn't throw with empty values
      expect(() => encodeJobArgs(job!, {})).not.toThrow();
    }
  });
});

// ── Cross-Blueprint Consistency ──

describe('Cross-Blueprint Consistency', () => {
  it('sandbox exec and instance exec produce different ABI shapes', () => {
    const sandboxJob = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.EXEC)!;
    const instanceJob = getJobById('ai-agent-instance-blueprint', INSTANCE_JOB_IDS.EXEC)!;

    // Sandbox exec has sidecar_url context param
    expect(sandboxJob.contextParams).toBeDefined();
    expect(sandboxJob.contextParams![0].abiName).toBe('sidecar_url');

    // Instance exec has no context params
    expect(instanceJob.contextParams).toBeUndefined();

    // Same form values produce different encoded bytes
    const sandboxEncoded = encodeJobArgs(sandboxJob, EXEC_VALUES, SIDECAR_CONTEXT);
    const instanceEncoded = encodeJobArgs(instanceJob, EXEC_VALUES);
    expect(sandboxEncoded).not.toBe(instanceEncoded);

    // But both decode correctly against their respective ABIs
    const sDec = decodeAbiParameters(SANDBOX_EXEC_ABI, sandboxEncoded);
    expect(sDec[0]).toBe('http://localhost:8080'); // sidecar_url
    expect(sDec[1]).toBe('ls -la /workspace');

    const iDec = decodeAbiParameters(INSTANCE_EXEC_ABI, instanceEncoded);
    expect(iDec[0]).toBe('ls -la /workspace'); // command is first (no sidecar_url)
  });

  it('TEE instance and standard instance produce identical ABI encoding for same values', () => {
    const stdJob = getJobById('ai-agent-instance-blueprint', INSTANCE_JOB_IDS.EXEC)!;
    const teeJob = getJobById('ai-agent-tee-instance-blueprint', INSTANCE_JOB_IDS.EXEC)!;

    const stdEncoded = encodeJobArgs(stdJob, EXEC_VALUES);
    const teeEncoded = encodeJobArgs(teeJob, EXEC_VALUES);
    expect(stdEncoded).toBe(teeEncoded);
  });

  it('field count matches Rust struct field count for all provision jobs', () => {
    // SandboxCreateRequest: 16 fields
    const sandboxCreate = getJobById('ai-agent-sandbox-blueprint', 0)!;
    expect(sandboxCreate.fields.filter(f => f.abiType).length).toBe(16);

    // ProvisionRequest: 17 fields (14 shared + sidecar_token + tee_required + tee_type)
    const instanceProvision = getJobById('ai-agent-instance-blueprint', 0)!;
    expect(instanceProvision.fields.filter(f => f.abiType).length).toBe(17);
  });
});
