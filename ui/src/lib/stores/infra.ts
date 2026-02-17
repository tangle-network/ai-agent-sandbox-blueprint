import { persistedAtom } from './persistedAtom';

/**
 * Infrastructure configuration store.
 * Auto-defaults from env vars; user can override via the Infrastructure modal.
 */

const defaultBlueprintId = import.meta.env.VITE_BLUEPRINT_ID ?? '0';
const defaultServiceId = import.meta.env.VITE_SERVICE_ID ?? import.meta.env.VITE_SERVICE_IDS?.split(',')[0] ?? '0';

export interface InfraConfig {
  blueprintId: string;
  serviceId: string;
  /** Whether the user has validated the service on-chain */
  serviceValidated: boolean;
  /** Cached service info from last validation */
  serviceInfo?: {
    active: boolean;
    operatorCount: number;
    owner: string;
    blueprintId: string;
    permitted: boolean;
  };
}

export const infraStore = persistedAtom<InfraConfig>({
  key: 'sandbox_cloud_infra',
  initial: {
    blueprintId: defaultBlueprintId,
    serviceId: defaultServiceId,
    serviceValidated: false,
  },
});

export function updateInfra(update: Partial<InfraConfig>) {
  infraStore.set({ ...infraStore.get(), ...update });
}

export function getInfra(): InfraConfig {
  return infraStore.get();
}
