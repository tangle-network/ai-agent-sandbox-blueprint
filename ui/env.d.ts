/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_WALLETCONNECT_PROJECT_ID: string | undefined;
  readonly VITE_TANGLE_CONTRACT: string | undefined;
  readonly VITE_SANDBOX_BSM: string | undefined;
  readonly VITE_INSTANCE_BSM: string | undefined;
  readonly VITE_TEE_INSTANCE_BSM: string | undefined;
  readonly VITE_SANDBOX_BLUEPRINT_ID: string | undefined;
  readonly VITE_INSTANCE_BLUEPRINT_ID: string | undefined;
  readonly VITE_TEE_INSTANCE_BLUEPRINT_ID: string | undefined;
  readonly VITE_SANDBOX_SERVICE_ID: string | undefined;
  readonly VITE_INSTANCE_SERVICE_ID: string | undefined;
  readonly VITE_OPERATOR_API_URL: string | undefined;
  readonly VITE_INSTANCE_OPERATOR_API_URL: string | undefined;
  // Testnet contract addresses
  readonly VITE_TESTNET_SANDBOX_BSM: string | undefined;
  readonly VITE_TESTNET_INSTANCE_BSM: string | undefined;
  readonly VITE_TESTNET_TEE_INSTANCE_BSM: string | undefined;
  readonly VITE_TESTNET_JOBS_ADDRESS: string | undefined;
  readonly VITE_TESTNET_SERVICES_ADDRESS: string | undefined;
  // Mainnet contract addresses
  readonly VITE_MAINNET_SANDBOX_BSM: string | undefined;
  readonly VITE_MAINNET_INSTANCE_BSM: string | undefined;
  readonly VITE_MAINNET_TEE_INSTANCE_BSM: string | undefined;
  readonly VITE_MAINNET_JOBS_ADDRESS: string | undefined;
  readonly VITE_MAINNET_SERVICES_ADDRESS: string | undefined;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
