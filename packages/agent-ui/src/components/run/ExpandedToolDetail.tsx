import { memo } from 'react';
import type { ToolPart } from '~/types/parts';
import { getToolDisplayMetadata } from '~/utils/toolDisplay';
import { CommandPreview } from '../tool-previews/CommandPreview';
import { WriteFilePreview } from '../tool-previews/WriteFilePreview';
import { CodeBlock } from '../markdown/CodeBlock';

export interface ExpandedToolDetailProps {
  part: ToolPart;
}

/** Format an unknown value as a displayable string. */
function formatOutput(value: unknown): string {
  if (value == null) return '';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/**
 * Renders full tool details when a tool item is expanded.
 * Dispatches to specialised previews (CommandPreview, WriteFilePreview)
 * or falls back to a generic input/output code view.
 */
export const ExpandedToolDetail = memo(({ part }: ExpandedToolDetailProps) => {
  const meta = getToolDisplayMetadata(part);
  const { status, input, output, error } = part.state;

  // Specialised previews
  if (meta.displayVariant === 'command') {
    return <CommandPreview part={part} />;
  }

  if (meta.displayVariant === 'write-file') {
    return <WriteFilePreview part={part} />;
  }

  // Generic fallback — show input/output/error
  const inputStr = formatOutput(input);
  const outputStr = formatOutput(output);

  return (
    <div className="space-y-2">
      {/* Input */}
      {inputStr && (
        <div className="rounded-lg overflow-hidden border border-neutral-200/50 dark:border-neutral-700/50">
          <div className="flex items-center gap-2 px-3 py-1.5 bg-neutral-100/80 dark:bg-neutral-800/80">
            <div className="i-ph:arrow-right w-3 h-3 text-neutral-400" />
            <span className="text-xs text-neutral-500 dark:text-neutral-400">Input</span>
          </div>
          <CodeBlock code={inputStr} language="json" className="rounded-none" />
        </div>
      )}

      {/* Output */}
      {status === 'completed' && outputStr && (
        <div className="rounded-lg overflow-hidden border border-neutral-200/50 dark:border-neutral-700/50">
          <div className="flex items-center gap-2 px-3 py-1.5 bg-neutral-100/80 dark:bg-neutral-800/80">
            <div className="i-ph:arrow-left w-3 h-3 text-neutral-400" />
            <span className="text-xs text-neutral-500 dark:text-neutral-400">Output</span>
          </div>
          <CodeBlock
            code={outputStr.length > 2000 ? outputStr.slice(0, 2000) + '\n...(truncated)' : outputStr}
            language="json"
            className="rounded-none"
          />
        </div>
      )}

      {/* Error */}
      {error && (
        <div className="rounded-lg overflow-hidden border border-red-300/50 dark:border-red-900/50">
          <div className="flex items-center gap-2 px-3 py-1.5 bg-red-100/30 dark:bg-red-900/30">
            <div className="i-ph:warning-circle w-3 h-3 text-red-500 dark:text-red-400" />
            <span className="text-xs text-red-500 dark:text-red-400">Error</span>
          </div>
          <pre className="p-3 text-xs font-mono text-red-600 dark:text-red-300 whitespace-pre-wrap break-all bg-neutral-50/60 dark:bg-neutral-900/60">
            {error}
          </pre>
        </div>
      )}

      {/* Running state */}
      {(status === 'pending' || status === 'running') && (
        <div className="flex items-center gap-2 px-3 py-2 text-xs text-neutral-400 dark:text-neutral-500">
          <div className="i-ph:circle-notch w-3 h-3 animate-spin" />
          Running...
        </div>
      )}
    </div>
  );
});
ExpandedToolDetail.displayName = 'ExpandedToolDetail';
