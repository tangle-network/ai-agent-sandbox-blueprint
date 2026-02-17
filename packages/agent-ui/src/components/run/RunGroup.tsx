import { memo, useMemo } from 'react';
import * as Collapsible from '@radix-ui/react-collapsible';
import { cn } from '~/utils/cn';
import { formatDuration } from '~/utils/format';
import type { Run, ToolCategory } from '~/types/run';
import type { SessionPart, ToolPart, ReasoningPart } from '~/types/parts';
import type { AgentBranding } from '~/types/branding';
import type { CustomToolRenderer } from '~/types/tool-display';
import { TOOL_CATEGORY_ICONS } from '~/utils/toolDisplay';
import { InlineToolItem } from './InlineToolItem';
import { InlineThinkingItem } from './InlineThinkingItem';
import { Markdown } from '../markdown/Markdown';

// ---------------------------------------------------------------------------
// Default branding
// ---------------------------------------------------------------------------

const DEFAULT_BRANDING: AgentBranding = {
  label: 'Agent',
  accentClass: 'text-blue-400',
  bgClass: 'bg-blue-500/10',
  containerBgClass: 'bg-neutral-900/40',
  borderClass: 'border-blue-500/20',
  iconClass: 'i-ph:robot',
  textClass: 'text-blue-400',
};

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface RunGroupProps {
  run: Run;
  partMap: Record<string, SessionPart[]>;
  collapsed: boolean;
  onToggle: () => void;
  branding?: AgentBranding;
  renderToolDetail?: CustomToolRenderer;
}

// ---------------------------------------------------------------------------
// Stat badges
// ---------------------------------------------------------------------------

function CategoryBadges({ categories }: { categories: Set<ToolCategory> }) {
  const sorted = useMemo(() => Array.from(categories).sort(), [categories]);
  if (sorted.length === 0) return null;

  return (
    <div className="flex items-center gap-1">
      {sorted.map((cat) => (
        <div
          key={cat}
          className={cn(TOOL_CATEGORY_ICONS[cat], 'w-3 h-3 text-neutral-500')}
          title={cat}
        />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * Collapsible container for a consecutive group of assistant messages (a "run").
 * Shows a summary header with stats and renders tool/thinking/text parts.
 */
export const RunGroup = memo(
  ({ run, partMap, collapsed, onToggle, branding = DEFAULT_BRANDING, renderToolDetail }: RunGroupProps) => {
    // Flatten all parts from all messages in this run
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

    const { stats, isStreaming } = run;

    return (
      <Collapsible.Root open={!collapsed} onOpenChange={() => onToggle()}>
        {/* Header */}
        <Collapsible.Trigger asChild>
          <button
            className={cn(
              'w-full flex items-center gap-2 px-3 py-2 text-left',
              'rounded-lg transition-colors',
              'hover:bg-neutral-800/60',
              branding.bgClass,
              collapsed && branding.borderClass && `border ${branding.borderClass}`,
              !collapsed && 'border border-transparent',
            )}
          >
            {/* Agent icon */}
            <div className={cn('w-4 h-4 shrink-0', branding.iconClass, branding.accentClass)} />

            {/* Label */}
            <span className={cn('text-xs font-medium shrink-0', branding.textClass)}>
              {branding.label}
            </span>

            {/* Stats */}
            <div className="flex items-center gap-3 flex-1 min-w-0">
              {stats.toolCount > 0 && (
                <span className="text-xs text-neutral-500">
                  {stats.toolCount} tool{stats.toolCount !== 1 ? 's' : ''}
                </span>
              )}
              {stats.thinkingDurationMs > 0 && (
                <span className="text-xs text-neutral-500">
                  {formatDuration(stats.thinkingDurationMs)} thinking
                </span>
              )}
              <CategoryBadges categories={stats.toolCategories} />
            </div>

            {/* Streaming indicator */}
            {isStreaming && (
              <div className="flex items-center gap-1.5">
                <div className="i-ph:circle-notch w-3 h-3 text-blue-400 animate-spin" />
                <span className="text-xs text-blue-400">Running</span>
              </div>
            )}

            {/* Collapse caret */}
            <div
              className={cn(
                'w-3.5 h-3.5 text-neutral-500 transition-transform shrink-0',
                !collapsed ? 'i-ph:caret-down' : 'i-ph:caret-right',
              )}
            />
          </button>
        </Collapsible.Trigger>

        {/* Summary text when collapsed */}
        {collapsed && run.summaryText && (
          <div className="px-3 py-2 text-xs text-neutral-400 line-clamp-2">
            {run.summaryText}
          </div>
        )}

        {/* Expanded content */}
        <Collapsible.Content className="overflow-hidden data-[state=open]:animate-slideDown data-[state=closed]:animate-slideUp">
          <div className={cn('mt-1 space-y-0.5 rounded-lg p-2', branding.containerBgClass)}>
            {allParts.map(({ part, msgId, index }) => {
              const key = `${msgId}-${index}`;

              if (part.type === 'tool') {
                return <InlineToolItem key={key} part={part as ToolPart} renderToolDetail={renderToolDetail} />;
              }

              if (part.type === 'reasoning') {
                return <InlineThinkingItem key={key} part={part as ReasoningPart} />;
              }

              if (part.type === 'text' && !part.synthetic && part.text.trim()) {
                return (
                  <div key={key} className="px-3 py-2">
                    <Markdown>{part.text}</Markdown>
                  </div>
                );
              }

              return null;
            })}
          </div>
        </Collapsible.Content>
      </Collapsible.Root>
    );
  },
);
RunGroup.displayName = 'RunGroup';
