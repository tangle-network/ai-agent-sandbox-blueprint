import { TEE_INSTANCE_PRICING_TIERS } from '~/lib/types/instance';
import { createInstanceJobs } from './instance-blueprint';
import { type BlueprintDefinition, registerBlueprint } from './registry';
import type { Address } from 'viem';

/**
 * TEE Instance Blueprint — identical to Instance but provisions through a TEE backend.
 * Higher pricing multipliers reflect TEE overhead. TEE fields default to enabled.
 */

export const TEE_INSTANCE_BLUEPRINT: BlueprintDefinition = {
  id: 'ai-agent-tee-instance-blueprint',
  name: 'AI Agent TEE Instance',
  version: '0.3.0',
  description: 'TEE-enforced AI agent instance with hardware-level isolation, attestation, and sealed secrets.',
  icon: 'i-ph:shield-check',
  color: 'violet',
  contracts: {},
  jobs: createInstanceJobs({
    pricingOverrides: Object.fromEntries(
      Object.entries(TEE_INSTANCE_PRICING_TIERS).map(([k, v]) => [Number(k), v.multiplier]),
    ),
    teeDefaults: true,
  }),
  categories: [
    { key: 'lifecycle', label: 'TEE Lifecycle', icon: 'i-ph:shield-check' },
    { key: 'execution', label: 'Execution', icon: 'i-ph:terminal' },
    { key: 'ssh', label: 'SSH Management', icon: 'i-ph:key' },
    { key: 'management', label: 'Management', icon: 'i-ph:gear' },
  ],
};

export function initTeeInstanceBlueprint(addressesByChain: Record<number, Address>) {
  TEE_INSTANCE_BLUEPRINT.contracts = addressesByChain;
  registerBlueprint(TEE_INSTANCE_BLUEPRINT);
}

registerBlueprint(TEE_INSTANCE_BLUEPRINT);
