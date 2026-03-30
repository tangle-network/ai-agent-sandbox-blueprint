import { useState, useMemo, useCallback, useEffect, useRef, type FormEvent, type KeyboardEvent } from 'react';
import { useStore } from '@nanostores/react';
import {
  useRunGroups,
  useRunCollapseState,
  useAutoScroll,
  UserMessage,
  RunGroup,
  Markdown,
  InlineToolItem,
  InlineThinkingItem,
  type AgentBranding,
  type SessionMessage,
  type SessionPart,
  type ToolPart,
  type ReasoningPart,
} from '@tangle-network/agent-ui';
import type { TextPart } from '@tangle-network/agent-ui';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import {
  chatSessionsStore,
  getSessions,
  getActiveSession,
  getLoading,
  getError,
  setActiveSession,
  fetchSessions,
  createSessionApi,
  deleteSessionApi,
  loadSessionDetail,
} from '~/lib/stores/chatSessions';
import { useSandboxSession } from '~/lib/hooks/useSandboxSession';
import { cn } from '@tangle-network/blueprint-ui';

// ---------------------------------------------------------------------------
// Branding
// ---------------------------------------------------------------------------

const CHAT_BRANDING: AgentBranding = {
  label: 'Agent',
  accentClass: 'text-teal-600 dark:text-teal-400',
  bgClass: 'bg-teal-500/5',
  containerBgClass: 'bg-teal-50/30 dark:bg-neutral-950/60',
  borderClass: 'border-teal-500/15 dark:border-teal-500/20',
  iconClass: 'i-ph:robot',
  textClass: 'text-teal-600 dark:text-teal-400',
};

// ---------------------------------------------------------------------------
// ChatArea — replaces ChatContainer with text-always-visible behavior
// ---------------------------------------------------------------------------

interface ChatAreaProps {
  messages: SessionMessage[];
  partMap: Record<string, SessionPart[]>;
  isStreaming: boolean;
  onSend?: (text: string) => void;
  branding: AgentBranding;
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
  run: import('@tangle-network/agent-ui').Run;
  partMap: Record<string, SessionPart[]>;
  collapsed: boolean;
  onToggle: () => void;
  branding: AgentBranding;
}) {
  const allParts = useMemo(() => {
    const parts: Array<{ part: SessionPart; msgId: string; index: number }> = [];
    for (const msg of run.messages) {
      const msgParts = partMap[msg.id] ?? [];
      msgParts.forEach((part, index) => {
        parts.push({ part, msgId: msg.id, index });
      });
    }
    return parts;
  }, [run.messages, partMap]);

  const textParts = useMemo(
    () =>
      allParts.filter(
        (p): p is { part: TextPart; msgId: string; index: number } =>
          p.part.type === 'text' && !(p.part as TextPart).synthetic && !!(p.part as TextPart).text.trim(),
      ),
    [allParts],
  );
  const collapsibleParts = useMemo(
    () => allParts.filter(({ part }) => part.type === 'tool' || part.type === 'reasoning'),
    [allParts],
  );
  const hasCollapsible = collapsibleParts.length > 0;

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
          {run.stats.toolCount > 0 && (
            <span className="text-xs text-neutral-400 dark:text-neutral-500">
              {run.stats.toolCount} tool{run.stats.toolCount !== 1 ? 's' : ''}
            </span>
          )}
        </div>
        {run.isStreaming && (
          <div className="flex items-center gap-1.5">
            <div className="i-ph:circle-notch w-3 h-3 text-teal-500 dark:text-teal-400 animate-spin" />
            <span className="text-xs text-teal-500 dark:text-teal-400">Running</span>
          </div>
        )}
        {hasCollapsible && (
          <div
            className={cn(
              'w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500 transition-transform shrink-0',
              !collapsed ? 'i-ph:caret-down' : 'i-ph:caret-right',
            )}
          />
        )}
      </button>

      {/* Text parts — always visible */}
      {textParts.length > 0 && (
        <div className={cn('mt-1 space-y-0.5 rounded-lg p-2', branding.containerBgClass)}>
          {textParts.map(({ part, msgId, index }) => (
            <div key={`${msgId}-${index}`} className="px-3 py-2">
              <Markdown>{part.text}</Markdown>
            </div>
          ))}
        </div>
      )}

      {/* Tool/thinking parts — collapsible */}
      {hasCollapsible && !collapsed && (
        <div className={cn('mt-1 space-y-0.5 rounded-lg p-2', branding.containerBgClass)}>
          {collapsibleParts.map(({ part, msgId, index }) => {
            const key = `${msgId}-${index}`;
            if (part.type === 'tool') {
              return <InlineToolItem key={key} part={part as ToolPart} />;
            }
            if (part.type === 'reasoning') {
              return <InlineThinkingItem key={key} part={part as ReasoningPart} />;
            }
            return null;
          })}
        </div>
      )}
    </div>
  );
}

function ChatArea({ messages, partMap, isStreaming, onSend, branding }: ChatAreaProps) {
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
                  <UserMessage
                    key={group.message.id}
                    message={group.message}
                    parts={partMap[group.message.id] ?? []}
                  />
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
          <div className="flex items-end gap-2">
            <textarea
              ref={inputRef}
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Ask the agent anything..."
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
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface SessionSidebarProps {
  sandboxId: string;
  client: SandboxClient | null;
  systemPrompt?: string;
  onSystemPromptChange?: (v: string) => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SessionSidebar({
  sandboxId,
  client,
  systemPrompt,
  onSystemPromptChange,
}: SessionSidebarProps) {
  // Subscribe to store changes
  const storeState = useStore(chatSessionsStore);
  const sessions = useMemo(() => getSessions(sandboxId), [storeState, sandboxId]);
  const activeSession = useMemo(() => getActiveSession(sandboxId), [storeState, sandboxId]);
  const isLoading = useMemo(() => getLoading(sandboxId), [storeState, sandboxId]);
  const storeError = useMemo(() => getError(sandboxId), [storeState, sandboxId]);

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSystemPrompt, setShowSystemPrompt] = useState(false);
  const [creating, setCreating] = useState(false);
  const fetchedRef = useRef(false);

  // Fetch sessions from API on mount
  useEffect(() => {
    if (!client || fetchedRef.current) return;
    fetchedRef.current = true;
    fetchSessions(client, sandboxId);
  }, [client, sandboxId]);

  // Load full session detail when active session changes and hasn't been loaded yet
  useEffect(() => {
    if (!client || !activeSession || activeSession.detailLoaded) return;
    void loadSessionDetail(client, sandboxId, activeSession.id);
  }, [client, sandboxId, activeSession?.id, activeSession?.detailLoaded]);

  // Poll while a run is active so refresh/reconnect still converges on the
  // durable session state even without a long-lived submit request.
  useEffect(() => {
    if (!client || !activeSession?.activeRunId) return;
    const interval = window.setInterval(() => {
      void loadSessionDetail(client, sandboxId, activeSession.id);
    }, 1500);
    return () => window.clearInterval(interval);
  }, [client, sandboxId, activeSession?.id, activeSession?.activeRunId]);

  // Session hook
  const { messages, partMap, isStreaming, error, send } = useSandboxSession({
    client,
    session: activeSession ?? null,
    sandboxId,
    systemPrompt,
  });

  const handleNewSession = useCallback(async () => {
    if (!client || creating) return;
    setCreating(true);
    await createSessionApi(client, sandboxId);
    setCreating(false);
  }, [client, sandboxId, creating]);

  const handleDelete = useCallback(
    async (e: React.MouseEvent, sessionId: string) => {
      e.stopPropagation();
      if (!client) return;
      await deleteSessionApi(client, sandboxId, sessionId);
    },
    [client, sandboxId],
  );

  const handleRetry = useCallback(() => {
    if (!client) return;
    fetchedRef.current = false;
    fetchSessions(client, sandboxId);
  }, [client, sandboxId]);

  // Loading state
  if (isLoading && sessions.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3 p-8">
        <div className="i-ph:spinner-gap text-2xl text-cloud-elements-textTertiary animate-spin" />
        <p className="text-sm text-cloud-elements-textSecondary">Loading sessions...</p>
      </div>
    );
  }

  // Error state (no sessions loaded)
  if (storeError && sessions.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 p-8">
        <div className="i-ph:warning-circle text-4xl text-crimson-500/60" />
        <p className="text-sm text-crimson-600 dark:text-crimson-400 text-center">{storeError}</p>
        <button
          onClick={handleRetry}
          className={cn(
            'flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-display font-medium',
            'bg-cloud-elements-item-backgroundHover border border-cloud-elements-borderColor',
            'text-cloud-elements-textPrimary hover:bg-cloud-elements-item-backgroundActive transition-colors',
          )}
        >
          <div className="i-ph:arrow-clockwise text-sm" />
          Retry
        </button>
      </div>
    );
  }

  // Empty state — no sessions yet
  if (sessions.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 p-8">
        <div className="i-ph:chat-circle text-4xl text-cloud-elements-textTertiary" />
        <p className="text-sm text-cloud-elements-textSecondary text-center">
          Start a conversation with the sandbox agent
        </p>
        <button
          onClick={handleNewSession}
          disabled={creating}
          className={cn(
            'flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-display font-medium',
            'bg-teal-500/10 border border-teal-500/20 text-teal-700 dark:text-teal-400',
            'hover:bg-teal-500/20 transition-colors',
            creating && 'opacity-50 cursor-not-allowed',
          )}
        >
          {creating ? (
            <div className="i-ph:spinner-gap text-sm animate-spin" />
          ) : (
            <div className="i-ph:plus text-sm" />
          )}
          New Chat
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0">
      {/* Session sidebar */}
      {sidebarOpen && (
        <div className="w-56 shrink-0 border-r border-cloud-elements-dividerColor/50 flex flex-col bg-cloud-elements-background-depth-1/50">
          {/* Header */}
          <div className="flex items-center justify-between px-3 py-2.5 border-b border-cloud-elements-dividerColor/50">
            <span className="text-xs font-display font-semibold text-cloud-elements-textSecondary uppercase tracking-wider">
              Sessions
            </span>
            <button
              onClick={handleNewSession}
              disabled={creating}
              className={cn(
                'flex items-center justify-center w-6 h-6 rounded-md hover:bg-cloud-elements-item-backgroundHover transition-colors',
                creating && 'opacity-50 cursor-not-allowed',
              )}
              title="New chat"
            >
              {creating ? (
                <div className="i-ph:spinner-gap text-sm text-cloud-elements-textTertiary animate-spin" />
              ) : (
                <div className="i-ph:plus text-sm text-cloud-elements-textTertiary" />
              )}
            </button>
          </div>

          {/* Session list */}
          <div className="flex-1 overflow-y-auto py-1">
            {sessions.map((s) => (
              <button
                key={s.id}
                onClick={() => setActiveSession(sandboxId, s.id)}
                className={cn(
                  'flex items-center gap-2 w-full px-3 py-2 text-left transition-colors group',
                  s.id === activeSession?.id
                    ? 'bg-cloud-elements-item-backgroundActive'
                    : 'hover:bg-cloud-elements-item-backgroundHover',
                )}
              >
                <div
                  className={cn(
                    'w-1.5 h-1.5 rounded-full shrink-0',
                    s.id === activeSession?.id ? 'bg-teal-500' : 'bg-cloud-elements-textTertiary/30',
                  )}
                />
                <span className="text-xs font-body text-cloud-elements-textPrimary truncate flex-1">
                  {s.title}
                </span>
                <button
                  onClick={(e) => handleDelete(e, s.id)}
                  className="opacity-0 group-hover:opacity-100 flex items-center justify-center w-5 h-5 rounded hover:bg-crimson-500/10 transition-all shrink-0"
                  title="Delete session"
                >
                  <div className="i-ph:x text-xs text-crimson-500" />
                </button>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Chat area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Toolbar */}
        <div className="flex items-center gap-2 px-3 py-2 border-b border-cloud-elements-dividerColor/50">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="flex items-center justify-center w-7 h-7 rounded-md hover:bg-cloud-elements-item-backgroundHover transition-colors"
            title={sidebarOpen ? 'Hide sessions' : 'Show sessions'}
          >
            <div className={cn('text-sm text-cloud-elements-textTertiary', sidebarOpen ? 'i-ph:sidebar-simple' : 'i-ph:list')} />
          </button>

          <span className="text-xs font-display font-medium text-cloud-elements-textPrimary truncate flex-1">
            {activeSession?.title ?? 'Chat'}
          </span>

          <button
            onClick={() => setShowSystemPrompt(!showSystemPrompt)}
            className={cn(
              'flex items-center gap-1.5 px-2 py-1 rounded-md text-xs transition-colors',
              showSystemPrompt
                ? 'bg-violet-500/10 text-violet-700 dark:text-violet-400'
                : 'text-cloud-elements-textTertiary hover:bg-cloud-elements-item-backgroundHover',
            )}
          >
            <div className="i-ph:gear text-xs" />
            System
          </button>
        </div>

        {/* System prompt (collapsible) */}
        {showSystemPrompt && onSystemPromptChange && (
          <div className="px-3 py-2 border-b border-cloud-elements-dividerColor/50 bg-cloud-elements-background-depth-1/30">
            <label className="block text-xs font-display text-cloud-elements-textTertiary mb-1.5">
              System Prompt
            </label>
            <textarea
              value={systemPrompt ?? ''}
              onChange={(e) => onSystemPromptChange(e.target.value)}
              placeholder="Optional system prompt for the agent..."
              rows={3}
              className={cn(
                'w-full resize-none rounded-lg px-3 py-2 text-xs',
                'bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor',
                'text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary',
                'focus:outline-none focus:border-violet-500/40',
              )}
            />
          </div>
        )}

        {/* Error banner */}
        {(error || storeError) && (
          <div className="px-3 py-2 bg-crimson-500/5 border-b border-crimson-500/20 flex items-center gap-2">
            <p className="text-xs text-crimson-600 dark:text-crimson-400 flex-1">{error || storeError}</p>
            {storeError && (
              <button
                onClick={handleRetry}
                className="text-xs text-crimson-600 dark:text-crimson-400 underline hover:no-underline shrink-0"
              >
                Retry
              </button>
            )}
          </div>
        )}

        {/* Messages loading indicator */}
        {activeSession && !activeSession.detailLoaded ? (
          <div className="flex-1 flex items-center justify-center">
            <div className="flex items-center gap-2 text-cloud-elements-textTertiary">
              <div className="i-ph:spinner-gap text-lg animate-spin" />
              <span className="text-sm">Loading chat...</span>
            </div>
          </div>
        ) : (
          /* Chat container — custom: text always visible, only tool/thinking collapsible */
          <ChatArea
            messages={messages}
            partMap={partMap}
            isStreaming={isStreaming}
            onSend={send}
            branding={CHAT_BRANDING}
          />
        )}
      </div>
    </div>
  );
}
