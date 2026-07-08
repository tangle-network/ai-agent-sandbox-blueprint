import type { ToolPart } from '@tangle-network/sandbox-ui/types';
import type { ChatRunSummary, ChatSessionDetail } from '~/lib/api/sandboxClient';
import type { AppSessionMessage } from '~/lib/types/chat';
import type { ChatRunProgressEntry, ChatSessionEntry } from './chatSessions.types';
import { mapRun, mapServerMessage, mapSessionPart } from './chatSessions.mappers';

export function applyStreamMessage(
  session: ChatSessionEntry,
  payload: ChatSessionDetail['messages'][number],
): ChatSessionEntry {
  const { message, parts } = mapServerMessage(payload, session.messages.length);
  const existingIndex = session.messages.findIndex((entry) => entry.id === message.id);
  const nextMessages = [...session.messages];
  if (existingIndex >= 0) {
    nextMessages[existingIndex] = message;
  } else {
    nextMessages.push(message);
  }

  return {
    ...session,
    messages: nextMessages,
    partMap: {
      ...session.partMap,
      [message.id]: parts,
    },
    detailLoaded: true,
  };
}

export function applyMessageUpdated(
  session: ChatSessionEntry,
  payload: Record<string, unknown>,
): ChatSessionEntry {
  const info = (payload.info as Record<string, unknown> | undefined) ?? payload;
  const id = typeof info.id === 'string' ? info.id : '';
  const role = typeof info.role === 'string' ? info.role : 'assistant';
  if (!id) {
    return session;
  }

  const time = (info.time as Record<string, unknown> | undefined) ?? {};
  const createdAt = typeof time.created === 'number'
    ? time.created
    : (typeof info.timestamp === 'number' ? info.timestamp : Date.now());
  const completedAt = typeof time.completed === 'number' ? time.completed : undefined;
  const runId = typeof info.runID === 'string'
    ? info.runID
    : (typeof info.run_id === 'string' ? info.run_id : undefined);
  const success = Object.prototype.hasOwnProperty.call(info, 'success')
    ? (typeof info.success === 'boolean' ? info.success : null)
    : undefined;
  const error = Object.prototype.hasOwnProperty.call(info, 'error')
    ? (typeof info.error === 'string' ? info.error : null)
    : undefined;
  const existingIndex = session.messages.findIndex((entry) => entry.id === id);
  const nextMessages = [...session.messages];
  const nextMessage: AppSessionMessage = {
    id,
    role: role as AppSessionMessage['role'],
    ...(runId !== undefined ? { runId } : {}),
    ...(success !== undefined ? { success } : {}),
    ...(error !== undefined ? { error } : {}),
    time: {
      created: createdAt,
      ...(completedAt ? { completed: completedAt } : {}),
    },
  };
  if (existingIndex >= 0) {
    nextMessages[existingIndex] = {
      ...nextMessages[existingIndex],
      ...nextMessage,
      time: nextMessage.time,
    };
  } else {
    nextMessages.push(nextMessage);
  }

  return {
    ...session,
    messages: nextMessages,
    detailLoaded: true,
  };
}

export function applyMessagePartUpdated(
  session: ChatSessionEntry,
  payload: Record<string, unknown>,
): ChatSessionEntry {
  const partPayload = (payload.part as Record<string, unknown> | undefined) ?? payload;
  const messageId = typeof partPayload.messageID === 'string'
    ? partPayload.messageID
    : (typeof payload.messageID === 'string' ? payload.messageID : '');
  if (!messageId) {
    return session;
  }

  const part = mapSessionPart(partPayload);
  if (!part) {
    return session;
  }

  const existingParts = session.partMap[messageId] ?? [];
  const nextParts = [...existingParts];
  let replaceIndex = -1;
  const partId = typeof partPayload.id === 'string' ? partPayload.id : undefined;

  if (partId) {
    replaceIndex = nextParts.findIndex((entry) => {
      if (!('id' in entry)) return false;
      return (entry as { id?: string }).id === partId;
    });
  } else if (part.type === 'tool') {
    replaceIndex = nextParts.findIndex(
      (entry) => entry.type === 'tool' && (entry as ToolPart).id === part.id,
    );
  } else if (part.type === 'text') {
    replaceIndex = nextParts.findIndex((entry) => entry.type === 'text');
  } else if (part.type === 'reasoning') {
    replaceIndex = nextParts.findIndex((entry) => entry.type === 'reasoning');
  }

  if (replaceIndex >= 0) {
    nextParts[replaceIndex] = part;
  } else {
    nextParts.push(part);
  }

  return {
    ...session,
    partMap: {
      ...session.partMap,
      [messageId]: nextParts,
    },
    detailLoaded: true,
  };
}

export function applyRunUpdate(
  session: ChatSessionEntry,
  payload: ChatRunSummary,
): ChatSessionEntry {
  const run = mapRun(payload);
  const existingIndex = session.runs.findIndex((entry) => entry.id === run.id);
  const runs = [...session.runs];
  if (existingIndex >= 0) {
    runs[existingIndex] = { ...runs[existingIndex], ...run };
  } else {
    runs.push(run);
  }

  const isActive = ['queued', 'running', 'cancelling'].includes(run.status);
  return {
    ...session,
    sidecarSessionId: payload.sidecar_session_id ?? session.sidecarSessionId,
    activeRunId: isActive ? run.id : (session.activeRunId === run.id ? undefined : session.activeRunId),
    runs,
    detailLoaded: true,
  };
}

export function applyRunProgress(
  session: ChatSessionEntry,
  payload: {
    run_id?: string;
    runId?: string;
    status?: string;
    phase?: string;
    message?: string;
    timestamp_ms?: number;
    timestampMs?: number;
  },
): ChatSessionEntry {
  const runId = payload.run_id ?? payload.runId;
  if (!runId) {
    return session;
  }

  const entry: ChatRunProgressEntry = {
    runId,
    status: payload.status ?? 'running',
    phase: payload.phase ?? 'progress',
    message: payload.message ?? '',
    timestampMs: payload.timestamp_ms ?? payload.timestampMs ?? Date.now(),
  };

  const deduped = session.runProgress.filter((item) => !(
    item.runId === entry.runId
    && item.phase === entry.phase
    && item.message === entry.message
    && item.timestampMs === entry.timestampMs
  ));

  return {
    ...session,
    runProgress: [...deduped, entry].slice(-50),
    detailLoaded: true,
  };
}
