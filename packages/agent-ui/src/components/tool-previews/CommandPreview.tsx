import { memo, useState } from 'react';
import { cn } from '~/utils/cn';
import type { ToolPart } from '~/types/parts';

export interface CommandPreviewProps {
  part: ToolPart;
}

/** Extract stdout/stderr/exitCode from tool output. */
function extractCommandOutput(output: unknown): {
  stdout: string;
  stderr: string;
  exitCode: number;
} {
  if (!output) return { stdout: '', stderr: '', exitCode: 0 };

  if (typeof output === 'string') return { stdout: output, stderr: '', exitCode: 0 };

  if (typeof output === 'object' && output !== null) {
    const obj = output as Record<string, unknown>;
    return {
      stdout: String(obj.stdout ?? obj.output ?? ''),
      stderr: String(obj.stderr ?? ''),
      exitCode: Number(obj.exitCode ?? obj.exit_code ?? obj.code ?? 0),
    };
  }

  return { stdout: String(output), stderr: '', exitCode: 0 };
}

/**
 * Terminal-style command output preview.
 * Shows the command, exit code, and stdout/stderr with expand/collapse.
 */
export const CommandPreview = memo(({ part }: CommandPreviewProps) => {
  const [expanded, setExpanded] = useState(true);
  const input = part.state.input as Record<string, unknown> | undefined;
  const command = typeof input?.command === 'string' ? input.command : String(input ?? '');

  const output =
    part.state.status === 'completed' ? extractCommandOutput(part.state.output) : null;

  const isError = output ? output.exitCode !== 0 : part.state.status === 'error';
  const errorText = part.state.error;

  return (
    <div className="rounded-lg overflow-hidden border border-neutral-700/50">
      {/* Command header */}
      <button
        onClick={() => setExpanded((e) => !e)}
        className="w-full flex items-center gap-2 px-3 py-2 bg-neutral-800/80 hover:bg-neutral-800 transition-colors text-left"
      >
        <div className="i-ph:terminal-window w-3.5 h-3.5 text-neutral-400 shrink-0" />
        <code className="text-xs font-mono text-neutral-200 truncate flex-1">{command}</code>
        {output && (
          <span
            className={cn(
              'text-xs font-mono px-1.5 py-0.5 rounded',
              isError ? 'bg-red-900/50 text-red-400' : 'bg-green-900/50 text-green-400',
            )}
          >
            {output.exitCode}
          </span>
        )}
        <div
          className={cn(
            'w-3 h-3 text-neutral-500 transition-transform',
            expanded ? 'i-ph:caret-down' : 'i-ph:caret-right',
          )}
        />
      </button>

      {/* Output body */}
      {expanded && (
        <div className="max-h-80 overflow-y-auto bg-neutral-900/60">
          {output?.stdout && (
            <pre className="p-3 text-xs font-mono text-neutral-300 whitespace-pre-wrap break-all">
              {output.stdout}
            </pre>
          )}
          {output?.stderr && (
            <pre className="p-3 text-xs font-mono text-red-400/80 whitespace-pre-wrap break-all border-t border-neutral-700/30">
              {output.stderr}
            </pre>
          )}
          {errorText && (
            <pre className="p-3 text-xs font-mono text-red-400 whitespace-pre-wrap break-all">
              {errorText}
            </pre>
          )}
          {part.state.status === 'running' && (
            <div className="p-3 flex items-center gap-2 text-xs text-neutral-500">
              <div className="i-ph:circle-notch w-3 h-3 animate-spin" />
              Running...
            </div>
          )}
        </div>
      )}
    </div>
  );
});
CommandPreview.displayName = 'CommandPreview';
