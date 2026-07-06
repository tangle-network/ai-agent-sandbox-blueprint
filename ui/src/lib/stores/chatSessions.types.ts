import type { SessionPart } from '@tangle-network/sandbox-ui/types';
import type { ChatRunSummary } from '~/lib/api/sandboxClient';
import type { AppSessionMessage } from '~/lib/types/chat';

export interface ChatRunEntry {
  id: string;
  kind: ChatRunSummary['kind'];
  status: ChatRunSummary['status'];
  requestText: string;
  createdAt: number;
  startedAt?: number;
  completedAt?: number;
  traceId?: string;
  finalOutput?: string;
  error?: string;
}

export interface ChatRunProgressEntry {
  runId: string;
  status: ChatRunEntry['status'] | string;
  phase: string;
  message: string;
  timestampMs: number;
}

export interface ChatSessionEntry {
  id: string;
  title: string;
  sandboxId: string;
  createdAt: number;
  sidecarSessionId?: string;
  activeRunId?: string;
  runs: ChatRunEntry[];
  runProgress: ChatRunProgressEntry[];
  messages: AppSessionMessage[];
  partMap: Record<string, SessionPart[]>;
  detailLoaded: boolean;
}
