import { memo, useCallback, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { cn } from '~/utils/cn';
import type { SessionMessage } from '~/types/message';
import type { SessionPart } from '~/types/parts';
import type { AgentBranding } from '~/types/branding';
import type { CustomToolRenderer } from '~/types/tool-display';
import { useRunGroups } from '~/hooks/useRunGroups';
import { useRunCollapseState } from '~/hooks/useRunCollapseState';
import { useAutoScroll } from '~/hooks/useAutoScroll';
import { MessageList } from './MessageList';

export interface ChatContainerProps {
  messages: SessionMessage[];
  partMap: Record<string, SessionPart[]>;
  isStreaming: boolean;
  onSend?: (text: string) => void;
  branding?: AgentBranding;
  placeholder?: string;
  className?: string;
  /** Hide the input area (useful for read-only views). */
  hideInput?: boolean;
  /** Custom renderer for tool details. Return ReactNode to override, null to use default. */
  renderToolDetail?: CustomToolRenderer;
}

/**
 * Full chat container: message list + auto-scroll + input area.
 * Orchestrates useRunGroups, useRunCollapseState, and useAutoScroll.
 */
export const ChatContainer = memo(
  ({
    messages,
    partMap,
    isStreaming,
    onSend,
    branding,
    placeholder = 'Type a message...',
    className,
    hideInput = false,
    renderToolDetail,
  }: ChatContainerProps) => {
    const [inputValue, setInputValue] = useState('');
    const scrollRef = useRef<HTMLDivElement>(null);
    const inputRef = useRef<HTMLTextAreaElement>(null);

    // Group messages into runs
    const groups = useRunGroups({ messages, partMap, isStreaming });

    // Extract runs for collapse state
    const runs = groups.filter((g) => g.type === 'run').map((g) => g.run);
    const { isCollapsed, toggleCollapse } = useRunCollapseState(runs);

    // Auto-scroll
    const { isAtBottom, scrollToBottom } = useAutoScroll(scrollRef, [messages, partMap, isStreaming]);

    const handleSubmit = useCallback(
      (e?: FormEvent) => {
        e?.preventDefault();
        const text = inputValue.trim();
        if (!text || !onSend) return;
        onSend(text);
        setInputValue('');
        // Re-focus input
        inputRef.current?.focus();
      },
      [inputValue, onSend],
    );

    const handleKeyDown = useCallback(
      (e: KeyboardEvent<HTMLTextAreaElement>) => {
        if (e.key === 'Enter' && !e.shiftKey) {
          e.preventDefault();
          handleSubmit();
        }
      },
      [handleSubmit],
    );

    return (
      <div className={cn('flex flex-col h-full', className)}>
        {/* Message area */}
        <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-4">
          {messages.length === 0 ? (
            <div className="flex items-center justify-center h-full text-sm text-neutral-400 dark:text-neutral-500">
              No messages yet
            </div>
          ) : (
            <MessageList
              groups={groups}
              partMap={partMap}
              isCollapsed={isCollapsed}
              onToggleCollapse={toggleCollapse}
              branding={branding}
              renderToolDetail={renderToolDetail}
            />
          )}
        </div>

        {/* Scroll-to-bottom button */}
        {!isAtBottom && (
          <div className="flex justify-center -mt-10 relative z-10">
            <button
              onClick={scrollToBottom}
              className={cn(
                'flex items-center gap-1.5 px-3 py-1.5 rounded-full',
                'bg-white dark:bg-neutral-800 border border-neutral-200 dark:border-neutral-700 shadow-lg',
                'text-xs text-neutral-600 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-700 transition-colors',
              )}
            >
              <div className="i-ph:arrow-down w-3 h-3" />
              Scroll to bottom
            </button>
          </div>
        )}

        {/* Input area */}
        {!hideInput && onSend && (
          <form onSubmit={handleSubmit} className="shrink-0 border-t border-neutral-200/50 dark:border-neutral-700/50 p-3">
            <div className="flex items-end gap-2">
              <textarea
                ref={inputRef}
                value={inputValue}
                onChange={(e) => setInputValue(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder={placeholder}
                rows={1}
                disabled={isStreaming}
                className={cn(
                  'flex-1 resize-none rounded-lg px-3 py-2',
                  'bg-neutral-50/60 dark:bg-neutral-800/60 border border-neutral-200/50 dark:border-neutral-700/50',
                  'text-sm text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 dark:placeholder:text-neutral-500',
                  'focus:outline-none focus:border-blue-500/50',
                  'disabled:opacity-50 disabled:cursor-not-allowed',
                  'max-h-32',
                )}
                style={{ minHeight: '2.5rem' }}
              />
              <button
                type="submit"
                disabled={isStreaming || !inputValue.trim()}
                className={cn(
                  'flex items-center justify-center w-9 h-9 rounded-lg',
                  'bg-blue-600 hover:bg-blue-500 transition-colors',
                  'disabled:opacity-30 disabled:cursor-not-allowed',
                )}
              >
                <div className="i-ph:paper-plane-tilt w-4 h-4 text-white" />
              </button>
            </div>
          </form>
        )}
      </div>
    );
  },
);
ChatContainer.displayName = 'ChatContainer';
