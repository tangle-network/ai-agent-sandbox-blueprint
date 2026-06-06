/** Shared environment-driven configuration constants. */

export const OPERATOR_API_URL =
  import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';

export const INSTANCE_OPERATOR_API_URL =
  import.meta.env.VITE_INSTANCE_OPERATOR_API_URL ?? '';

const LOCAL_NETWORK_ENABLED = import.meta.env.VITE_ENABLE_LOCAL_NETWORK === 'true';

function fromBaseOrLocal(base: string | undefined, local: string | undefined, fallback: string) {
  return base ?? (LOCAL_NETWORK_ENABLED ? local : undefined) ?? fallback;
}

export const SANDBOX_ONCHAIN_BLUEPRINT_ID = fromBaseOrLocal(
  import.meta.env.VITE_BASE_SEPOLIA_SANDBOX_BLUEPRINT_ID,
  import.meta.env.VITE_SANDBOX_BLUEPRINT_ID,
  '10',
);

export const INSTANCE_ONCHAIN_BLUEPRINT_ID = fromBaseOrLocal(
  import.meta.env.VITE_BASE_SEPOLIA_INSTANCE_BLUEPRINT_ID,
  import.meta.env.VITE_INSTANCE_BLUEPRINT_ID,
  '11',
);

export const TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID = fromBaseOrLocal(
  import.meta.env.VITE_BASE_SEPOLIA_TEE_INSTANCE_BLUEPRINT_ID,
  import.meta.env.VITE_TEE_INSTANCE_BLUEPRINT_ID,
  '12',
);

export const SANDBOX_ONCHAIN_SERVICE_ID = fromBaseOrLocal(
  import.meta.env.VITE_BASE_SEPOLIA_SANDBOX_SERVICE_ID,
  import.meta.env.VITE_SANDBOX_SERVICE_ID,
  '1',
);

export const INSTANCE_ONCHAIN_SERVICE_ID = fromBaseOrLocal(
  import.meta.env.VITE_BASE_SEPOLIA_INSTANCE_SERVICE_ID,
  import.meta.env.VITE_INSTANCE_SERVICE_ID,
  '2',
);

export const TEE_INSTANCE_ONCHAIN_SERVICE_ID =
  import.meta.env.VITE_BASE_SEPOLIA_TEE_INSTANCE_SERVICE_ID
  ?? (LOCAL_NETWORK_ENABLED ? import.meta.env.VITE_TEE_INSTANCE_SERVICE_ID : undefined)
  ?? '';
