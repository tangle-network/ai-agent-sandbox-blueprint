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

export type JobCategory = 'lifecycle' | 'execution' | 'batch' | 'workflow' | 'ssh';

export interface JobFieldDef {
  name: string;
  label: string;
  type: 'text' | 'textarea' | 'number' | 'boolean' | 'select' | 'json';
  placeholder?: string;
  required?: boolean;
  defaultValue?: string | number | boolean;
  options?: { label: string; value: string }[];
  helperText?: string;
}

export interface JobDefinition {
  id: number;
  name: string;
  label: string;
  description: string;
  category: JobCategory;
  icon: string;
  pricingMultiplier: number;
  /** Fields the user needs to fill for this job, beyond the sandbox ID */
  fields: JobFieldDef[];
  /** Whether this job requires an existing sandbox to target */
  requiresSandbox: boolean;
  /** Optional warning shown before submission */
  warning?: string;
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
