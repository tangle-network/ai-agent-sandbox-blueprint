import { useState, useMemo, useCallback, useRef, type FormEvent, type KeyboardEvent } from 'react';
import {
  type AgentBranding,
  type SessionPart,
  type Run,
} from '@tangle-network/sandbox-ui/types';
import {
  useAutoScroll,
  useRunCollapseState,
  useRunGroups,
} from '@tangle-network/sandbox-ui/hooks';
import type { AppSessionMessage } from '~/lib/types/chat';
import { cn } from '@tangle-network/blueprint-ui';
import { AppMarkdown, ReasoningRow, ToolRow, UserBubble } from './SessionChatParts';
import { collectSessionTimelineParts, collectVisibleSessionTimelineParts } from './sessionChatTimeline';

function getRunFailureState(run: Run): { errorText: string | null } | null {
  for (let index = run.messages.length - 1; index >= 0; index -= 1) {
    const message = run.messages[index] as AppSessionMessage;
    if (message.role !== 'assistant') {
      continue;
    }
    if (message.success === false || typeof message.error === 'string') {
      return {
        errorText: typeof message.error === 'string' ? message.error : null,
      };
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// ChatArea — replaces ChatContainer with text-always-visible behavior
// ---------------------------------------------------------------------------

interface ChatAreaProps {
  messages: AppSessionMessage[];
  partMap: Record<string, SessionPart[]>;
  isStreaming: boolean;
  onSend?: (text: string) => void;
  branding: AgentBranding;
  inputDisabled?: boolean;
  inputDisabledReason?: string;
  onSelectLiveSession?: () => void;
}

/**
 * Custom run group that keeps text parts always visible and only
 * collapses tool/thinking parts behind the banner.
 */
function AgentRunGroup({
  run,
  partMap,
  collapsed,
  onToggle,
  branding,
}: {
  run: Run;
  partMap: Record<string, SessionPart[]>;
  collapsed: boolean;
  onToggle: () => void;
  branding: AgentBranding;
}) {
  const allParts = useMemo(
    () => collectSessionTimelineParts(run.messages, partMap),
    [run.messages, partMap],
  );
  const visibleParts = useMemo(
    () => collectVisibleSessionTimelineParts(run.messages, partMap, collapsed),
    [collapsed, partMap, run.messages],
  );
  const hasCollapsible = useMemo(
    () => allParts.some(({ part }) => part.type === 'tool' || part.type === 'reasoning'),
    [allParts],
  );
  const hasVisibleParts = visibleParts.length > 0;
  const failureState = useMemo(() => getRunFailureState(run), [run]);

  if (!hasVisibleParts && !run.isStreaming && !failureState) {
    return null;
  }

  return (
    <div>
      {/* Header banner */}
      <button
        onClick={hasCollapsible ? onToggle : undefined}
        className={cn(
          'w-full flex items-center gap-2 px-3 py-2 text-left',
          'rounded-lg transition-colors',
          branding.bgClass,
          hasCollapsible && 'hover:bg-neutral-100/60 dark:hover:bg-neutral-800/60 cursor-pointer',
          !hasCollapsible && 'cursor-default',
          collapsed && branding.borderClass && `border ${branding.borderClass}`,
          !collapsed && 'border border-transparent',
        )}
      >
        <div className={cn('w-4 h-4 shrink-0', branding.iconClass, branding.accentClass)} />
        <span className={cn('text-xs font-medium shrink-0', branding.textClass)}>
          {branding.label}
        </span>
        <div className="flex items-center gap-3 flex-1 min-w-0">
          {failureState && (
            <span className="rounded-full bg-crimson-500/10 px-2 py-0.5 text-[11px] font-medium text-crimson-600 dark:text-crimson-400">
              Failed
            </span>
          )}
          {run.stats.toolCount > 0 && (
            <span className="text-xs text-neutral-400 dark:text-neutral-500">
              {run.stats.toolCount} tool{run.stats.toolCount !== 1 ? 's' : ''}
            </span>
          )}
        </div>
        {hasCollapsible && (
          <div
            className={cn(
              'w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500 transition-transform shrink-0',
              !collapsed ? 'i-ph:caret-down' : 'i-ph:caret-right',
            )}
          />
        )}
      </button>

      {/* Run timeline — text stays visible when collapsed, full chronology when expanded */}
      {visibleParts.length > 0 && (
        <div className={cn('mt-1 space-y-2.5 rounded-lg p-2', branding.containerBgClass)}>
          {visibleParts.map(({ part, msgId, index }) => {
            const key = `${msgId}-${index}`;
            if (part.type === 'text') {
              return (
                <div key={key} className="px-3 py-2">
                  <AppMarkdown className="text-[14px] leading-6">{part.text}</AppMarkdown>
                </div>
              );
            }
            if (part.type === 'tool') {
              return <ToolRow key={key} part={part} />;
            }
            if (part.type === 'reasoning') {
              return <ReasoningRow key={key} part={part} />;
            }
            return null;
          })}
        </div>
      )}
      {failureState && (
        <div className="mt-2 rounded-lg border border-crimson-500/20 bg-crimson-500/5 px-3 py-2">
          <div className="text-xs font-medium text-crimson-600 dark:text-crimson-400">
            Generation stopped due to an error. This response may be incomplete.
          </div>
          {failureState.errorText && (
            <p className="mt-1 text-xs text-crimson-600/90 dark:text-crimson-400/90">
              {failureState.errorText}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

export function ChatArea({
  messages,
  partMap,
  isStreaming,
  onSend,
  branding,
  inputDisabled = false,
  inputDisabledReason,
  onSelectLiveSession,
}: ChatAreaProps) {
  const [inputValue, setInputValue] = useState('');
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const groups = useRunGroups({ messages, partMap, isStreaming });
  const runs = groups.filter((g) => g.type === 'run').map((g) => g.run);
  const { isCollapsed, toggleCollapse } = useRunCollapseState(runs);
  const { isAtBottom, scrollToBottom } = useAutoScroll(scrollRef, [messages, partMap, isStreaming]);

  const handleSubmit = useCallback(
    (e?: FormEvent) => {
      e?.preventDefault();
      const text = inputValue.trim();
      if (!text || !onSend) return;
      onSend(text);
      setInputValue('');
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

  const composerDisabled = isStreaming || inputDisabled;

  return (
    <div className="flex flex-col h-full flex-1 min-h-0">
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-4">
        {messages.length === 0 ? (
          <div className="flex items-center justify-center h-full text-sm text-neutral-400 dark:text-neutral-500">
            No messages yet
          </div>
        ) : (
          <div className="space-y-3">
            {groups.map((group) => {
              if (group.type === 'user') {
                return (
                  <UserBubble key={group.message.id} parts={partMap[group.message.id] ?? []} />
                );
              }
              return (
                <AgentRunGroup
                  key={group.run.id}
                  run={group.run}
                  partMap={partMap}
                  collapsed={isCollapsed(group.run.id)}
                  onToggle={() => toggleCollapse(group.run.id)}
                  branding={branding}
                />
              );
            })}
          </div>
        )}
      </div>

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

      {onSend && (
        <form onSubmit={handleSubmit} className="shrink-0 border-t border-neutral-200/50 dark:border-neutral-700/50 p-3">
          {inputDisabledReason && (
            <div className="mb-2 flex items-center gap-2 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2">
              <span className="text-xs text-amber-700 dark:text-amber-400 flex-1">
                {inputDisabledReason}
              </span>
              {onSelectLiveSession && (
                <button
                  type="button"
                  onClick={onSelectLiveSession}
                  className="text-xs font-medium text-amber-700 dark:text-amber-400 underline hover:no-underline"
                >
                  Switch
                </button>
              )}
            </div>
          )}
          <div className="flex items-end gap-2">
            <textarea
              ref={inputRef}
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={inputDisabled ? 'Another chat session is currently live...' : 'Ask the agent anything...'}
              rows={1}
              disabled={composerDisabled}
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
              disabled={composerDisabled || !inputValue.trim()}
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
}
