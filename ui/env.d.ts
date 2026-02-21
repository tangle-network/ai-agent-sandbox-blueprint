/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_RPC_URL: string;
  readonly VITE_CHAIN_ID: string;
  readonly VITE_WALLETCONNECT_PROJECT_ID: string;
  readonly VITE_TANGLE_CONTRACT: string;
  readonly VITE_SANDBOX_BSM: string;
  readonly VITE_INSTANCE_BSM: string;
  readonly VITE_TEE_INSTANCE_BSM: string;
  readonly VITE_SANDBOX_BLUEPRINT_ID: string;
  readonly VITE_INSTANCE_BLUEPRINT_ID: string;
  readonly VITE_TEE_INSTANCE_BLUEPRINT_ID: string;
  readonly VITE_SANDBOX_SERVICE_ID: string;
  readonly VITE_INSTANCE_SERVICE_ID: string;
  readonly VITE_OPERATOR_API_URL: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
