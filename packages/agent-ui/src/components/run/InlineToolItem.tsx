import { memo, useCallback, useEffect, useRef, useState } from 'react';
import * as Collapsible from '@radix-ui/react-collapsible';
import { cn } from '~/utils/cn';
import { getToolDisplayMetadata, getToolErrorText } from '~/utils/toolDisplay';
import { formatDuration } from '~/utils/format';
import type { ToolPart } from '~/types/parts';
import type { CustomToolRenderer } from '~/types/tool-display';
import { ExpandedToolDetail } from './ExpandedToolDetail';

export interface InlineToolItemProps {
  part: ToolPart;
  renderToolDetail?: CustomToolRenderer;
}

/** Live timer that updates every 100ms while a tool is running. */
function StreamingTimer({ startTime }: { startTime: number }) {
  const [elapsed, setElapsed] = useState(Date.now() - startTime);

  useEffect(() => {
    const id = setInterval(() => setElapsed(Date.now() - startTime), 100);
    return () => clearInterval(id);
  }, [startTime]);

  return (
    <span className="text-xs font-mono text-neutral-500 tabular-nums">
      {formatDuration(elapsed)}
    </span>
  );
}

/**
 * Compact single-line tool call display (32px height).
 * Shows icon, title, description, duration, and status indicator.
 * Expands on click to show ExpandedToolDetail.
 */
export const InlineToolItem = memo(({ part, renderToolDetail }: InlineToolItemProps) => {
  const [open, setOpen] = useState(false);
  const meta = getToolDisplayMetadata(part);
  const { status } = part.state;
  const errorText = getToolErrorText(part);

  const isRunning = status === 'pending' || status === 'running';
  const isError = status === 'error';
  const isComplete = status === 'completed';

  // Duration
  const startTime = part.state.time?.start;
  const endTime = part.state.time?.end;
  const durationMs = startTime && endTime ? endTime - startTime : undefined;

  return (
    <Collapsible.Root open={open} onOpenChange={setOpen}>
      <Collapsible.Trigger asChild>
        <button
          className={cn(
            'w-full flex items-center gap-2 px-3 py-1.5 text-left',
            'rounded-md transition-colors text-xs',
            'hover:bg-neutral-800/60',
            open && 'bg-neutral-800/40',
          )}
        >
          {/* Status / Icon */}
          <div
            className={cn(
              'w-4 h-4 shrink-0',
              isRunning && 'i-ph:circle-notch animate-spin text-blue-400',
              isComplete && 'i-ph:check-circle text-green-400',
              isError && 'i-ph:warning-circle text-red-400',
              !isRunning && !isComplete && !isError && `${meta.iconClass} text-neutral-400`,
            )}
          />

          {/* Title + description */}
          <span className="font-medium text-neutral-200 shrink-0">{meta.title}</span>
          {meta.description && (
            <span className="text-neutral-500 truncate flex-1 font-mono">
              {meta.description}
            </span>
          )}
          {!meta.description && <span className="flex-1" />}

          {/* Duration or streaming timer */}
          {isRunning && startTime && <StreamingTimer startTime={startTime} />}
          {!isRunning && durationMs != null && (
            <span className="text-xs font-mono text-neutral-500 tabular-nums">
              {formatDuration(durationMs)}
            </span>
          )}

          {/* Error indicator */}
          {errorText && (
            <span className="text-xs text-red-400 truncate max-w-32">{errorText}</span>
          )}

          {/* Caret */}
          <div
            className={cn(
              'w-3 h-3 text-neutral-500 transition-transform shrink-0',
              open ? 'i-ph:caret-down' : 'i-ph:caret-right',
            )}
          />
        </button>
      </Collapsible.Trigger>

      <Collapsible.Content className="overflow-hidden data-[state=open]:animate-slideDown data-[state=closed]:animate-slideUp">
        <div className="ml-6 mt-1 mb-2">
          {renderToolDetail?.(part) ?? <ExpandedToolDetail part={part} />}
        </div>
      </Collapsible.Content>
    </Collapsible.Root>
  );
});
InlineToolItem.displayName = 'InlineToolItem';
