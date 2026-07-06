import type { ReasoningPart, SessionPart, TextPart, ToolPart } from '@tangle-network/sandbox-ui/types';
import type {
  ChatRunSummary,
  ChatSessionDetail,
  ChatSessionSummary,
} from '~/lib/api/sandboxClient';
import type { AppSessionMessage } from '~/lib/types/chat';
import type { ChatRunEntry, ChatRunProgressEntry, ChatSessionEntry } from './chatSessions.types';

export function mapServerMessage(
  msg: ChatSessionDetail['messages'][number],
  index: number,
): { message: AppSessionMessage; parts: SessionPart[] } {
  const createdAt = typeof msg.created_at === 'number' ? msg.created_at : Date.now();
  const parts = mapServerParts(msg.parts, msg.content ?? '');
  const success = typeof msg.success === 'boolean' ? msg.success : msg.success === null ? null : undefined;
  const error = typeof msg.error === 'string' ? msg.error : msg.error === null ? null : undefined;
  return {
    message: {
      id: msg.id ?? `server-${index}`,
      role: msg.role as 'user' | 'assistant' | 'system',
      ...(typeof msg.run_id === 'string' ? { runId: msg.run_id } : {}),
      ...(success !== undefined ? { success } : {}),
      ...(error !== undefined ? { error } : {}),
      time: {
        created: createdAt,
        ...(typeof msg.completed_at === 'number' ? { completed: msg.completed_at } : {}),
      },
    },
    parts,
  };
}

function mapServerParts(
  rawParts: Array<Record<string, unknown>> | undefined,
  fallbackText: string,
): SessionPart[] {
  const mapped = (rawParts ?? [])
    .map(mapSessionPart)
    .filter((part): part is SessionPart => part !== null);
  if (mapped.length > 0) {
    return mapped;
  }
  if (!fallbackText) {
    return [];
  }
  return [{ type: 'text', text: fallbackText } satisfies TextPart];
}

function mapToolState(state: Record<string, unknown> | undefined): ToolPart['state'] {
  const status = state?.status === 'failed'
    ? 'error'
    : (state?.status as ToolPart['state']['status'] | undefined);
  return {
    status: status ?? 'running',
    input: state?.input,
    output: state?.output,
    error: typeof state?.error === 'string' ? state.error : undefined,
    metadata: (state?.metadata as Record<string, unknown> | undefined),
    time: (state?.time as ToolPart['state']['time'] | undefined),
  };
}

export function mapSessionPart(rawPart: Record<string, unknown>): SessionPart | null {
  const type = typeof rawPart.type === 'string' ? rawPart.type : '';
  if (type === 'tool') {
    return {
      type: 'tool',
      id: typeof rawPart.id === 'string' ? rawPart.id : `tool-${Date.now()}`,
      tool: typeof rawPart.tool === 'string' ? rawPart.tool : 'unknown',
      state: mapToolState(rawPart.state as Record<string, unknown> | undefined),
    } satisfies ToolPart;
  }
  if (type === 'reasoning') {
    return {
      type: 'reasoning',
      ...(rawPart.id && typeof rawPart.id === 'string' ? { id: rawPart.id } : {}),
      text: typeof rawPart.text === 'string' ? rawPart.text : '',
      time: rawPart.time as ReasoningPart['time'] | undefined,
    } satisfies ReasoningPart;
  }
  if (type === 'text') {
    return {
      type: 'text',
      text: typeof rawPart.text === 'string' ? rawPart.text : '',
      ...(rawPart.id && typeof rawPart.id === 'string' ? { id: rawPart.id } : {}),
    } as TextPart;
  }
  return null;
}

export function mapRun(run: ChatRunSummary): ChatRunEntry {
  return {
    id: run.id,
    kind: run.kind,
    status: run.status,
    requestText: run.request_text,
    createdAt: run.created_at,
    startedAt: run.started_at ?? undefined,
    completedAt: run.completed_at ?? undefined,
    traceId: run.trace_id ?? undefined,
    finalOutput: run.final_output ?? undefined,
    error: run.error ?? undefined,
  };
}

function mapRunProgress(
  entry: NonNullable<ChatSessionDetail['run_progress']>[number],
): ChatRunProgressEntry | null {
  const runId = entry.run_id ?? undefined;
  if (!runId) {
    return null;
  }

  return {
    runId,
    status: entry.status ?? 'running',
    phase: entry.phase ?? 'progress',
    message: entry.message ?? '',
    timestampMs: entry.timestamp_ms ?? Date.now(),
  };
}

export function applySessionSummary(
  sandboxId: string,
  existing: ChatSessionEntry | undefined,
  summary: ChatSessionSummary,
): ChatSessionEntry {
  return {
    id: summary.session_id,
    title: summary.title,
    sandboxId,
    createdAt: existing?.createdAt ?? Date.now(),
    sidecarSessionId: existing?.sidecarSessionId,
    activeRunId: summary.active_run_id ?? existing?.activeRunId,
    runs: existing?.runs ?? [],
    runProgress: existing?.runProgress ?? [],
    messages: existing?.messages ?? [],
    partMap: existing?.partMap ?? {},
    detailLoaded: existing?.detailLoaded ?? false,
  };
}

export function applySessionDetail(
  session: ChatSessionEntry,
  detail: ChatSessionDetail,
): ChatSessionEntry {
  const converted = (detail.messages ?? []).map((message, index) => mapServerMessage(message, index));
  const messages = converted.map((entry) => entry.message);
  const partMap: Record<string, SessionPart[]> = {};
  for (const entry of converted) {
    partMap[entry.message.id] = entry.parts;
  }

  return {
    ...session,
    title: detail.title,
    sidecarSessionId: detail.sidecar_session_id ?? undefined,
    activeRunId: detail.active_run_id ?? undefined,
    runs: (detail.runs ?? []).map(mapRun),
    runProgress: (detail.run_progress ?? [])
      .map((entry) => mapRunProgress(entry))
      .filter((entry): entry is ChatRunProgressEntry => entry !== null),
    messages,
    partMap,
    detailLoaded: true,
  };
}
