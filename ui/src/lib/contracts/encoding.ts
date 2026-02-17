import { encodeAbiParameters, decodeAbiParameters, type Address } from 'viem';
import type { SandboxCreateParams } from '~/lib/types/sandbox';

/**
 * Encoding helpers for Tangle blueprint job arguments.
 * Each job expects ABI-encoded bytes as its input. These helpers
 * produce the encoded `args` param for `submitJob(serviceId, job, args)`.
 */

// ── Sandbox Lifecycle ──

export function encodeSandboxCreate(params: SandboxCreateParams): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'name', type: 'string' },
      { name: 'image', type: 'string' },
      { name: 'stack', type: 'string' },
      { name: 'agentIdentifier', type: 'string' },
      { name: 'envJson', type: 'string' },
      { name: 'metadataJson', type: 'string' },
      { name: 'sshEnabled', type: 'bool' },
      { name: 'sshPublicKey', type: 'string' },
      { name: 'webTerminalEnabled', type: 'bool' },
      { name: 'maxLifetimeSeconds', type: 'uint64' },
      { name: 'idleTimeoutSeconds', type: 'uint64' },
      { name: 'cpuCores', type: 'uint32' },
      { name: 'memoryMb', type: 'uint32' },
      { name: 'diskGb', type: 'uint32' },
    ],
    [
      params.name,
      params.image,
      params.stack,
      params.agentIdentifier,
      params.envJson,
      params.metadataJson,
      params.sshEnabled,
      params.sshPublicKey,
      params.webTerminalEnabled,
      BigInt(params.maxLifetimeSeconds),
      BigInt(params.idleTimeoutSeconds),
      params.cpuCores,
      params.memoryMb,
      params.diskGb,
    ],
  );
}

export function encodeSandboxId(sandboxId: string): `0x${string}` {
  return encodeAbiParameters([{ name: 'sandboxId', type: 'string' }], [sandboxId]);
}

export function encodeSnapshot(sandboxId: string, tier: string, destination?: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxId', type: 'string' },
      { name: 'tier', type: 'string' },
      { name: 'destination', type: 'string' },
    ],
    [sandboxId, tier, destination ?? ''],
  );
}

// ── Execution ──

export function encodeExec(sandboxId: string, command: string, args: string[] = []): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxId', type: 'string' },
      { name: 'command', type: 'string' },
      { name: 'args', type: 'string[]' },
    ],
    [sandboxId, command, args],
  );
}

export function encodePrompt(sandboxId: string, prompt: string, systemPrompt?: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxId', type: 'string' },
      { name: 'prompt', type: 'string' },
      { name: 'systemPrompt', type: 'string' },
    ],
    [sandboxId, prompt, systemPrompt ?? ''],
  );
}

export function encodeTask(sandboxId: string, task: string, systemPrompt?: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxId', type: 'string' },
      { name: 'task', type: 'string' },
      { name: 'systemPrompt', type: 'string' },
    ],
    [sandboxId, task, systemPrompt ?? ''],
  );
}

// ── Batch Operations ──

export function encodeBatchCreate(count: number, configJson: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'count', type: 'uint32' },
      { name: 'configJson', type: 'string' },
    ],
    [count, configJson],
  );
}

export function encodeBatchExec(sandboxIds: string[], command: string, args: string[] = []): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxIds', type: 'string[]' },
      { name: 'command', type: 'string' },
      { name: 'args', type: 'string[]' },
    ],
    [sandboxIds, command, args],
  );
}

export function encodeBatchTask(sandboxIds: string[], task: string, systemPrompt?: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxIds', type: 'string[]' },
      { name: 'task', type: 'string' },
      { name: 'systemPrompt', type: 'string' },
    ],
    [sandboxIds, task, systemPrompt ?? ''],
  );
}

export function encodeBatchCollect(batchId: string): `0x${string}` {
  return encodeAbiParameters([{ name: 'batchId', type: 'string' }], [batchId]);
}

// ── Workflows ──

export function encodeWorkflowCreate(
  name: string,
  workflowJson: string,
  triggerType: string,
  triggerConfig: string,
  sandboxConfigJson: string,
): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'name', type: 'string' },
      { name: 'workflowJson', type: 'string' },
      { name: 'triggerType', type: 'string' },
      { name: 'triggerConfig', type: 'string' },
      { name: 'sandboxConfigJson', type: 'string' },
    ],
    [name, workflowJson, triggerType, triggerConfig, sandboxConfigJson],
  );
}

export function encodeWorkflowControl(workflowId: bigint): `0x${string}` {
  return encodeAbiParameters([{ name: 'workflowId', type: 'uint64' }], [workflowId]);
}

// ── SSH ──

export function encodeSshProvision(sandboxId: string, publicKey: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxId', type: 'string' },
      { name: 'publicKey', type: 'string' },
    ],
    [sandboxId, publicKey],
  );
}

export function encodeSshRevoke(sandboxId: string, publicKey: string): `0x${string}` {
  return encodeAbiParameters(
    [
      { name: 'sandboxId', type: 'string' },
      { name: 'publicKey', type: 'string' },
    ],
    [sandboxId, publicKey],
  );
}

// ── Result Decoding ──

export function decodeJobResult(data: `0x${string}`): { success: boolean; output: string } {
  try {
    const [success, output] = decodeAbiParameters(
      [
        { name: 'success', type: 'bool' },
        { name: 'output', type: 'string' },
      ],
      data,
    );
    return { success, output };
  } catch {
    return { success: false, output: 'Failed to decode result' };
  }
}

export function decodeSandboxCreateResult(data: `0x${string}`): { sandboxId: string; sidecarUrl: string } {
  try {
    const [sandboxId, sidecarUrl] = decodeAbiParameters(
      [
        { name: 'sandboxId', type: 'string' },
        { name: 'sidecarUrl', type: 'string' },
      ],
      data,
    );
    return { sandboxId, sidecarUrl };
  } catch {
    return { sandboxId: '', sidecarUrl: '' };
  }
}
