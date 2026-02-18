import { encodeAbiParameters } from 'viem';
import type { JobDefinition } from '~/lib/blueprints/registry';

/**
 * Generic ABI encoder for any blueprint job.
 *
 * Builds the ordered parameter array from:
 *   1. contextParams (e.g. sidecar_url, sandbox_id) — prepended first
 *   2. fields with abiType set — appended in definition order
 *
 * Delegates to job.customEncoder when present (for nested-struct jobs like batch_create).
 */
export function encodeJobArgs(
  job: JobDefinition,
  formValues: Record<string, unknown>,
  context?: Record<string, unknown>,
): `0x${string}` {
  if (job.customEncoder) {
    return job.customEncoder(formValues, context);
  }

  const abiDefs: { name: string; type: string }[] = [];
  const values: unknown[] = [];

  // Context params first (sidecar_url, sandbox_id, etc.)
  if (job.contextParams) {
    for (const cp of job.contextParams) {
      abiDefs.push({ name: cp.abiName, type: cp.abiType });
      values.push(coerceValue(context?.[cp.abiName], cp.abiType));
    }
  }

  // Form fields with ABI metadata
  for (const field of job.fields) {
    if (!field.abiType) continue;
    abiDefs.push({ name: field.abiParam ?? field.name, type: field.abiType });
    values.push(coerceValue(formValues[field.name], field.abiType));
  }

  return encodeAbiParameters(abiDefs, values);
}

function coerceValue(value: unknown, abiType: string): unknown {
  switch (abiType) {
    case 'bool':
      return Boolean(value);
    case 'uint8':
    case 'uint16':
    case 'uint32':
      return Number(value) || 0;
    case 'uint64':
    case 'uint128':
    case 'uint256':
      return BigInt(Number(value) || 0);
    case 'string':
      return String(value ?? '');
    case 'string[]':
      if (Array.isArray(value)) return value.map(String);
      return String(value ?? '').split('\n').filter(Boolean);
    case 'address[]':
      if (Array.isArray(value)) return value;
      return String(value ?? '')
        .split('\n')
        .map((s) => s.trim())
        .filter((s) => /^0x[a-fA-F0-9]{40}$/.test(s));
    default:
      return value;
  }
}
