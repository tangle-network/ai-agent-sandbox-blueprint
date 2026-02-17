/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_RPC_URL: string;
  readonly VITE_CHAIN_ID: string;
  readonly VITE_WALLETCONNECT_PROJECT_ID: string;
  readonly VITE_SANDBOX_CONTRACT: string;
  readonly VITE_JOBS_CONTRACT: string;
  readonly VITE_SERVICES_CONTRACT: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
