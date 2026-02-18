import type { Address } from 'viem';

/**
 * Blueprint Registry — defines the metadata layer for Tangle blueprints.
 *
 * Each blueprint exposes a set of jobs. The registry maps on-chain job IDs
 * to human-readable metadata: labels, descriptions, categories, form fields,
 * and pricing info. This enables the UI to render appropriate forms for each
 * job without procedurally generating UI from raw ABI data.
 *
 * Third-party blueprints can register here to appear in the wizard.
 */

// ── Types ──

export type JobCategory = 'lifecycle' | 'execution' | 'batch' | 'workflow' | 'ssh' | 'management';

export interface JobFieldDef {
  name: string;
  label: string;
  type: 'text' | 'textarea' | 'number' | 'boolean' | 'select' | 'json';
  placeholder?: string;
  required?: boolean;
  defaultValue?: string | number | boolean;
  options?: { label: string; value: string }[];
  helperText?: string;
  /** Solidity ABI type for encoding (e.g. 'string', 'uint64', 'bool', 'uint8') */
  abiType?: string;
  /** ABI param name if different from `name` (e.g. 'agent_identifier' vs 'agentIdentifier') */
  abiParam?: string;
  /** Field is included in ABI encoding but never shown in form (e.g. sidecar_token) */
  internal?: boolean;
}

/** ABI param injected from runtime context, not user input (e.g. sidecar_url, sandbox_id) */
export interface AbiContextParam {
  abiName: string;
  abiType: string;
}

export interface JobDefinition {
  id: number;
  name: string;
  label: string;
  description: string;
  category: JobCategory;
  icon: string;
  pricingMultiplier: number;
  /** Fields the user needs to fill for this job */
  fields: JobFieldDef[];
  /** Whether this job requires an existing sandbox to target */
  requiresSandbox: boolean;
  /** Optional warning shown before submission */
  warning?: string;
  /** ABI params prepended from runtime context (e.g. sidecar_url for sandbox jobs) */
  contextParams?: AbiContextParam[];
  /** Override for jobs with complex ABI encoding (e.g. nested structs) */
  customEncoder?: (values: Record<string, unknown>, context?: Record<string, unknown>) => `0x${string}`;
}

export interface BlueprintDefinition {
  id: string;
  name: string;
  version: string;
  description: string;
  icon: string;
  color: string;
  /** Contract address per chain ID — resolved at runtime */
  contracts: Record<number, Address>;
  /** Supported job definitions */
  jobs: JobDefinition[];
  /** Category ordering for the UI */
  categories: { key: JobCategory; label: string; icon: string }[];
}

// ── Registry ──

const blueprintRegistry = new Map<string, BlueprintDefinition>();

export function registerBlueprint(bp: BlueprintDefinition) {
  blueprintRegistry.set(bp.id, bp);
}

export function getBlueprint(id: string): BlueprintDefinition | undefined {
  return blueprintRegistry.get(id);
}

export function getAllBlueprints(): BlueprintDefinition[] {
  return Array.from(blueprintRegistry.values());
}

export function getBlueprintJobs(blueprintId: string, category?: JobCategory): JobDefinition[] {
  const bp = blueprintRegistry.get(blueprintId);
  if (!bp) return [];
  return category ? bp.jobs.filter((j) => j.category === category) : bp.jobs;
}

export function getJobById(blueprintId: string, jobId: number): JobDefinition | undefined {
  const bp = blueprintRegistry.get(blueprintId);
  return bp?.jobs.find((j) => j.id === jobId);
}
