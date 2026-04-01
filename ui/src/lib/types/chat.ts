import type { SessionMessage as SandboxSessionMessage } from '@tangle-network/sandbox-ui/types';

export type AppSessionMessage = SandboxSessionMessage & {
  runId?: string;
  success?: boolean | null;
  error?: string | null;
};
