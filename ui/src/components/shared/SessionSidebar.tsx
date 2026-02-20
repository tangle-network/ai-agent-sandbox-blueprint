import { useState, useMemo, useCallback } from 'react';
import { useStore } from '@nanostores/react';
import { ChatContainer, type AgentBranding } from '@tangle/agent-ui';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import {
  chatSessionsStore,
  getSessions,
  getActiveSession,
  setActiveSession,
  createSession,
  deleteSession,
} from '~/lib/stores/chatSessions';
import { useSandboxSession } from '~/lib/hooks/useSandboxSession';
import { cn } from '~/lib/utils';

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

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSystemPrompt, setShowSystemPrompt] = useState(false);

  // Session hook
  const { messages, partMap, isStreaming, error, send } = useSandboxSession({
    client,
    session: activeSession ?? null,
    sandboxId,
    systemPrompt,
  });

  const handleNewSession = useCallback(() => {
    createSession(sandboxId);
  }, [sandboxId]);

  const handleDelete = useCallback(
    (e: React.MouseEvent, sessionId: string) => {
      e.stopPropagation();
      deleteSession(sandboxId, sessionId);
    },
    [sandboxId],
  );

  // Auto-create a session if none exist
  if (sessions.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 p-8">
        <div className="i-ph:chat-circle text-4xl text-cloud-elements-textTertiary" />
        <p className="text-sm text-cloud-elements-textSecondary text-center">
          Start a conversation with the sandbox agent
        </p>
        <button
          onClick={handleNewSession}
          className={cn(
            'flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-display font-medium',
            'bg-teal-500/10 border border-teal-500/20 text-teal-700 dark:text-teal-400',
            'hover:bg-teal-500/20 transition-colors',
          )}
        >
          <div className="i-ph:plus text-sm" />
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
              className="flex items-center justify-center w-6 h-6 rounded-md hover:bg-cloud-elements-item-backgroundHover transition-colors"
              title="New chat"
            >
              <div className="i-ph:plus text-sm text-cloud-elements-textTertiary" />
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
            <div className={cn('text-sm text-cloud-elements-textTertiary', sidebarOpen ? 'i-ph:sidebar-simple' : 'i-ph:sidebar-simple')} />
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
        {error && (
          <div className="px-3 py-2 bg-crimson-500/5 border-b border-crimson-500/20">
            <p className="text-xs text-crimson-600 dark:text-crimson-400">{error}</p>
          </div>
        )}

        {/* Chat container */}
        <div className="flex-1 min-h-0">
          <ChatContainer
            messages={messages}
            partMap={partMap}
            isStreaming={isStreaming}
            onSend={send}
            branding={CHAT_BRANDING}
            placeholder="Ask the agent anything..."
          />
        </div>
      </div>
    </div>
  );
}
