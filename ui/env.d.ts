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
  readonly VITE_INSTANCE_OPERATOR_API_URL: string;
  // Testnet contract addresses
  readonly VITE_TESTNET_SANDBOX_BSM: string;
  readonly VITE_TESTNET_INSTANCE_BSM: string;
  readonly VITE_TESTNET_TEE_INSTANCE_BSM: string;
  readonly VITE_TESTNET_JOBS_ADDRESS: string;
  readonly VITE_TESTNET_SERVICES_ADDRESS: string;
  // Mainnet contract addresses
  readonly VITE_MAINNET_SANDBOX_BSM: string;
  readonly VITE_MAINNET_INSTANCE_BSM: string;
  readonly VITE_MAINNET_TEE_INSTANCE_BSM: string;
  readonly VITE_MAINNET_JOBS_ADDRESS: string;
  readonly VITE_MAINNET_SERVICES_ADDRESS: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
