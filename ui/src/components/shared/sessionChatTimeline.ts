import type { SessionMessage, SessionPart, TextPart } from '@tangle-network/sandbox-ui';

export interface SessionTimelineEntry {
  part: SessionPart;
  msgId: string;
  index: number;
}

export function isRenderableTextPart(part: SessionPart): part is TextPart {
  return part.type === 'text' && !part.synthetic && part.text.trim().length > 0;
}

export function collectSessionTimelineParts(
  messages: SessionMessage[],
  partMap: Record<string, SessionPart[]>,
): SessionTimelineEntry[] {
  const parts: SessionTimelineEntry[] = [];

  for (const msg of messages) {
    const msgParts = partMap[msg.id] ?? [];
    msgParts.forEach((part, index) => {
      if (part.type === 'text' && !isRenderableTextPart(part)) {
        return;
      }

      parts.push({ part, msgId: msg.id, index });
    });
  }

  return parts;
}

export function collectVisibleSessionTimelineParts(
  messages: SessionMessage[],
  partMap: Record<string, SessionPart[]>,
  collapsed: boolean,
): SessionTimelineEntry[] {
  const allParts = collectSessionTimelineParts(messages, partMap);
  if (!collapsed) {
    return allParts;
  }

  return allParts.filter(({ part }) => isRenderableTextPart(part));
}
