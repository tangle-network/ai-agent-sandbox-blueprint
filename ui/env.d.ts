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
  readonly VITE_ENABLE_LOCAL_NETWORK: string | undefined;
  readonly VITE_DEPLOYMENT_FINGERPRINT: string | undefined;
  readonly VITE_TEE_INSTANCE_SERVICE_ID: string | undefined;
  // Base Sepolia contract addresses
  readonly VITE_BASE_SEPOLIA_RPC_URL: string | undefined;
  readonly VITE_BASE_SEPOLIA_TANGLE_CONTRACT: string | undefined;
  readonly VITE_BASE_SEPOLIA_SANDBOX_BSM: string | undefined;
  readonly VITE_BASE_SEPOLIA_INSTANCE_BSM: string | undefined;
  readonly VITE_BASE_SEPOLIA_TEE_INSTANCE_BSM: string | undefined;
  readonly VITE_BASE_SEPOLIA_SANDBOX_BLUEPRINT_ID: string | undefined;
  readonly VITE_BASE_SEPOLIA_INSTANCE_BLUEPRINT_ID: string | undefined;
  readonly VITE_BASE_SEPOLIA_TEE_INSTANCE_BLUEPRINT_ID: string | undefined;
  readonly VITE_BASE_SEPOLIA_SANDBOX_SERVICE_ID: string | undefined;
  readonly VITE_BASE_SEPOLIA_INSTANCE_SERVICE_ID: string | undefined;
  readonly VITE_BASE_SEPOLIA_TEE_INSTANCE_SERVICE_ID: string | undefined;
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
