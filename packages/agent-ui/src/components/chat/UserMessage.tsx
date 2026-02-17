import { memo } from 'react';
import type { SessionMessage } from '~/types/message';
import type { SessionPart } from '~/types/parts';
import { Markdown } from '../markdown/Markdown';

export interface UserMessageProps {
  message: SessionMessage;
  parts: SessionPart[];
}

/**
 * Simple user message bubble.
 * Renders text parts from the user's message.
 */
export const UserMessage = memo(({ message, parts }: UserMessageProps) => {
  const textContent = parts
    .filter((p) => p.type === 'text')
    .map((p) => (p as { text: string }).text)
    .join('\n');

  if (!textContent.trim()) return null;

  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-2xl rounded-br-md px-4 py-2.5 bg-blue-600/20 border border-blue-500/20">
        <div className="text-sm text-neutral-100">
          <Markdown>{textContent}</Markdown>
        </div>
      </div>
    </div>
  );
});
UserMessage.displayName = 'UserMessage';
