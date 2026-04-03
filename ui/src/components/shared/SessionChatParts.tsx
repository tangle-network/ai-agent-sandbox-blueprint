import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  type ReasoningPart,
  type SessionPart,
  type TextPart,
  type ToolPart,
} from '@tangle-network/sandbox-ui/types';
import {
  formatDuration,
  getToolCategory,
  getToolDisplayMetadata,
  getToolErrorText,
  truncateText,
} from '@tangle-network/sandbox-ui/utils';
import { cn } from '@tangle-network/blueprint-ui';

const TOOL_CATEGORY_ICON_CLASS: Record<string, string> = {
  command: 'i-ph:terminal-window',
  write: 'i-ph:file-plus',
  read: 'i-ph:file-text',
  search: 'i-ph:magnifying-glass',
  edit: 'i-ph:pencil-line',
  task: 'i-ph:cpu',
  web: 'i-ph:globe-hemisphere-west',
  todo: 'i-ph:check-square',
  other: 'i-ph:cube',
};

function renderInlineMarkdown(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\((https?:\/\/[^\s)]+)\))/g;
  let lastIndex = 0;

  for (const match of text.matchAll(pattern)) {
    const full = match[0];
    const index = match.index ?? 0;

    if (index > lastIndex) {
      nodes.push(text.slice(lastIndex, index));
    }

    if (full.startsWith('`') && full.endsWith('`')) {
      nodes.push(
        <code
          key={`${index}-code`}
          className="rounded-md border border-white/8 bg-white/5 px-1.5 py-0.5 font-data text-[0.92em] text-teal-200"
        >
          {full.slice(1, -1)}
        </code>,
      );
    } else if (full.startsWith('**') && full.endsWith('**')) {
      nodes.push(
        <strong key={`${index}-strong`} className="font-semibold text-cloud-elements-textPrimary">
          {full.slice(2, -2)}
        </strong>,
      );
    } else {
      const textMatch = /^\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)$/.exec(full);
      if (textMatch) {
        nodes.push(
          <a
            key={`${index}-link`}
            href={textMatch[2]}
            target="_blank"
            rel="noreferrer"
            className="text-teal-500 underline decoration-teal-500/40 underline-offset-3 hover:text-teal-400"
          >
            {textMatch[1]}
          </a>,
        );
      } else {
        nodes.push(full);
      }
    }

    lastIndex = index + full.length;
  }

  if (lastIndex < text.length) {
    nodes.push(text.slice(lastIndex));
  }

  return nodes.length > 0 ? nodes : [text];
}

function renderTextWithBreaks(text: string) {
  return text.split('\n').map((line, index) => (
    <Fragment key={`${line}-${index}`}>
      {index > 0 ? <br /> : null}
      {renderInlineMarkdown(line)}
    </Fragment>
  ));
}

function renderCodeBlock(code: string, language?: string) {
  return (
    <pre className="overflow-x-auto rounded-xl border border-white/8 bg-black/25 px-3 py-2.5 font-data text-[11px] leading-5 text-cloud-elements-textPrimary">
      <code className={language ? `language-${language}` : undefined}>{code}</code>
    </pre>
  );
}

function LiveDuration({ startTime }: { startTime: number }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <span className="rounded-full border border-teal-400/20 bg-teal-500/10 px-1.5 py-0.5 text-[10px] font-data text-teal-600 dark:text-teal-300">
      {formatDuration(Math.max(0, now - startTime))}
    </span>
  );
}

function renderToolValue(value: unknown): ReactNode {
  if (value == null) {
    return <span className="text-cloud-elements-textTertiary">None</span>;
  }

  if (typeof value === 'string') {
    return (
      <pre className="overflow-x-auto whitespace-pre-wrap rounded-lg bg-black/20 px-2.5 py-2 text-[11px] leading-5 text-cloud-elements-textSecondary">
        {value}
      </pre>
    );
  }

  return (
    <pre className="overflow-x-auto whitespace-pre-wrap rounded-lg bg-black/20 px-2.5 py-2 text-[11px] leading-5 text-cloud-elements-textSecondary">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}

function DetailSection({
  label,
  value,
  tone = 'neutral',
}: {
  label: string;
  value: unknown;
  tone?: 'neutral' | 'error';
}) {
  return (
    <div
      className={cn(
        'rounded-xl border px-2.5 py-2.5',
        tone === 'error'
          ? 'border-crimson-500/20 bg-crimson-500/5'
          : 'border-cloud-elements-dividerColor/50 bg-cloud-elements-background-depth-2/60',
      )}
    >
      <div
        className={cn(
          'mb-1.5 text-[10px] font-display font-semibold uppercase tracking-[0.12em]',
          tone === 'error' ? 'text-crimson-500/80' : 'text-cloud-elements-textTertiary',
        )}
      >
        {label}
      </div>
      {renderToolValue(value)}
    </div>
  );
}

export function AppMarkdown({
  children,
  className,
}: {
  children: string;
  className?: string;
}) {
  const lines = children.replace(/\r\n/g, '\n').split('\n');
  const blocks: ReactNode[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    const trimmed = line.trim();

    if (!trimmed) {
      index += 1;
      continue;
    }

    const fenceMatch = /^```(\w+)?\s*$/.exec(trimmed);
    if (fenceMatch) {
      const codeLines: string[] = [];
      index += 1;
      while (index < lines.length && !/^```/.test(lines[index].trim())) {
        codeLines.push(lines[index]);
        index += 1;
      }
      if (index < lines.length) {
        index += 1;
      }
      blocks.push(<div key={`code-${blocks.length}`}>{renderCodeBlock(codeLines.join('\n'), fenceMatch[1])}</div>);
      continue;
    }

    const headingMatch = /^(#{1,3})\s+(.+)$/.exec(trimmed);
    if (headingMatch) {
      const level = headingMatch[1].length;
      const headingClass =
        level === 1
          ? 'text-lg font-display font-semibold text-cloud-elements-textPrimary'
          : level === 2
            ? 'text-base font-display font-semibold text-cloud-elements-textPrimary'
            : 'text-sm font-display font-semibold uppercase tracking-[0.12em] text-cloud-elements-textSecondary';
      blocks.push(
        <div key={`heading-${blocks.length}`} className={headingClass}>
          {renderInlineMarkdown(headingMatch[2])}
        </div>,
      );
      index += 1;
      continue;
    }

    if (trimmed.startsWith('>')) {
      const quoteLines: string[] = [];
      while (index < lines.length && lines[index].trim().startsWith('>')) {
        quoteLines.push(lines[index].trim().replace(/^>\s?/, ''));
        index += 1;
      }
      blocks.push(
        <blockquote
          key={`quote-${blocks.length}`}
          className="border-l-2 border-teal-400/40 pl-4 text-cloud-elements-textSecondary italic"
        >
          {renderTextWithBreaks(quoteLines.join('\n'))}
        </blockquote>,
      );
      continue;
    }

    if (/^[-*]\s+/.test(trimmed)) {
      const items: string[] = [];
      while (index < lines.length && /^[-*]\s+/.test(lines[index].trim())) {
        items.push(lines[index].trim().replace(/^[-*]\s+/, ''));
        index += 1;
      }
      blocks.push(
        <ul key={`ul-${blocks.length}`} className="list-disc space-y-2 pl-5 text-cloud-elements-textPrimary">
          {items.map((item, itemIndex) => (
            <li key={`${item}-${itemIndex}`} className="marker:text-teal-400">
              {renderInlineMarkdown(item)}
            </li>
          ))}
        </ul>,
      );
      continue;
    }

    if (/^\d+\.\s+/.test(trimmed)) {
      const items: string[] = [];
      while (index < lines.length && /^\d+\.\s+/.test(lines[index].trim())) {
        items.push(lines[index].trim().replace(/^\d+\.\s+/, ''));
        index += 1;
      }
      blocks.push(
        <ol key={`ol-${blocks.length}`} className="list-decimal space-y-2 pl-5 text-cloud-elements-textPrimary">
          {items.map((item, itemIndex) => (
            <li key={`${item}-${itemIndex}`} className="marker:text-teal-400">
              {renderInlineMarkdown(item)}
            </li>
          ))}
        </ol>,
      );
      continue;
    }

    const paragraphLines: string[] = [];
    while (
      index < lines.length &&
      lines[index].trim() &&
      !/^```/.test(lines[index].trim()) &&
      !/^(#{1,3})\s+/.test(lines[index].trim()) &&
      !/^>\s?/.test(lines[index].trim()) &&
      !/^[-*]\s+/.test(lines[index].trim()) &&
      !/^\d+\.\s+/.test(lines[index].trim())
    ) {
      paragraphLines.push(lines[index]);
      index += 1;
    }

    blocks.push(
      <p key={`p-${blocks.length}`} className="text-[15px] leading-7 text-cloud-elements-textPrimary">
        {renderTextWithBreaks(paragraphLines.join('\n'))}
      </p>,
    );
  }

  return <div className={cn('space-y-3 text-[15px] leading-7 text-cloud-elements-textPrimary', className)}>{blocks}</div>;
}

export function UserBubble({ parts }: { parts: SessionPart[] }) {
  const textContent = parts
    .filter((part): part is TextPart => part.type === 'text')
    .map((part) => part.text)
    .join('\n');

  if (!textContent.trim()) {
    return null;
  }

  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-2xl rounded-br-md border border-blue-500/15 bg-blue-600/10 px-4 py-2.5 dark:border-blue-500/20 dark:bg-blue-600/20">
        <div className="mb-1 text-[11px] font-display font-semibold uppercase tracking-[0.14em] text-blue-500 dark:text-blue-300">
          You
        </div>
        <AppMarkdown>{textContent}</AppMarkdown>
      </div>
    </div>
  );
}

export function ToolRow({ part }: { part: ToolPart }) {
  const [open, setOpen] = useState(false);
  const meta = getToolDisplayMetadata(part);
  const errorText = getToolErrorText(part);
  const status = part.state.status;
  const isRunning = status === 'pending' || status === 'running';
  const isComplete = status === 'completed';
  const isError = status === 'error';
  const startTime = part.state.time?.start;
  const endTime = part.state.time?.end;
  const durationMs = startTime && endTime ? endTime - startTime : undefined;
  const category = getToolCategory(part.tool);
  const iconClass = TOOL_CATEGORY_ICON_CLASS[category] ?? TOOL_CATEGORY_ICON_CLASS.other;

  return (
    <div className="rounded-xl border border-cloud-elements-dividerColor/50 bg-cloud-elements-background-depth-2/70">
      <button
        type="button"
        onClick={() => setOpen((current) => !current)}
        className={cn(
          'w-full rounded-xl px-2.5 py-2.5 text-left transition-colors',
          'hover:bg-cloud-elements-item-backgroundHover/60',
          open && 'bg-cloud-elements-item-backgroundHover/40',
        )}
      >
        <div className="flex items-center gap-2.5">
          <div
            className={cn(
              'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
              isRunning && 'border-teal-400/30 bg-teal-500/10 text-teal-300',
              isComplete && 'border-emerald-400/25 bg-emerald-500/10 text-emerald-300',
              isError && 'border-crimson-400/25 bg-crimson-500/10 text-crimson-300',
              !isRunning && !isComplete && !isError && 'border-white/8 bg-white/5 text-cloud-elements-textTertiary',
            )}
          >
            <div className={cn('h-3.5 w-3.5', isRunning ? 'i-ph:spinner-gap animate-spin' : iconClass)} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span className="truncate text-[13px] font-display font-medium text-cloud-elements-textPrimary">
                {meta.title}
              </span>
              {isRunning && (
                <span className="rounded-full border border-teal-400/20 bg-teal-500/10 px-1.5 py-0.5 text-[10px] font-display font-semibold uppercase tracking-[0.06em] text-teal-300">
                  Running
                </span>
              )}
              {isError && (
                <span className="rounded-full border border-crimson-400/20 bg-crimson-500/10 px-1.5 py-0.5 text-[10px] font-display font-semibold uppercase tracking-[0.06em] text-crimson-300">
                  Failed
                </span>
              )}
            </div>
            {meta.description && (
              <div className="mt-0.5 truncate text-[11px] font-data text-cloud-elements-textTertiary">
                {meta.description}
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {isRunning && startTime ? <LiveDuration startTime={startTime} /> : null}
            {!isRunning && durationMs != null ? (
              <span className="rounded-full border border-white/8 bg-white/5 px-1.5 py-0.5 text-[10px] font-data text-cloud-elements-textTertiary">
                {formatDuration(durationMs)}
              </span>
            ) : null}
            <div className={cn('h-3.5 w-3.5 text-cloud-elements-textTertiary', open ? 'i-ph:caret-down' : 'i-ph:caret-right')} />
          </div>
        </div>
        {errorText && !open && (
          <div className="mt-2 rounded-lg border border-crimson-500/20 bg-crimson-500/5 px-2.5 py-2 text-[11px] text-crimson-300">
            {errorText}
          </div>
        )}
      </button>

      {open && (
        <div className="space-y-2.5 border-t border-cloud-elements-dividerColor/50 px-2.5 py-2.5">
          {part.state.input !== undefined && <DetailSection label="Input" value={part.state.input} />}
          {part.state.output !== undefined && <DetailSection label="Output" value={part.state.output} />}
          {errorText && <DetailSection label="Error" value={errorText} tone="error" />}
        </div>
      )}
    </div>
  );
}

export function ReasoningRow({
  part,
  defaultOpen = false,
}: {
  part: ReasoningPart;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const autoCollapsedRef = useRef(false);
  const startTime = part.time?.start;
  const endTime = part.time?.end;
  const durationMs = startTime && endTime ? endTime - startTime : undefined;
  const isActive = startTime != null && endTime == null;
  const preview = useMemo(() => (part.text ? truncateText(part.text, 120) : undefined), [part.text]);

  useEffect(() => {
    if (isActive) {
      autoCollapsedRef.current = false;
      setOpen(true);
      return;
    }

    if (!autoCollapsedRef.current && durationMs != null) {
      const timer = window.setTimeout(() => {
        setOpen(false);
        autoCollapsedRef.current = true;
      }, 900);
      return () => {
        window.clearTimeout(timer);
      };
    }
  }, [durationMs, isActive]);

  return (
    <div className="rounded-xl border border-cloud-elements-dividerColor/50 bg-cloud-elements-background-depth-2/70">
      <button
        type="button"
        onClick={() => setOpen((current) => !current)}
        className={cn(
          'w-full rounded-xl px-2.5 py-2.5 text-left transition-colors',
          'hover:bg-cloud-elements-item-backgroundHover/60',
          open && 'bg-cloud-elements-item-backgroundHover/40',
        )}
      >
        <div className="flex items-center gap-2.5">
          <div
            className={cn(
              'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
              isActive
                ? 'border-teal-400/30 bg-teal-500/10 text-teal-300 shadow-[0_0_20px_rgba(45,212,191,0.14)]'
                : 'border-white/8 bg-white/5 text-cloud-elements-textTertiary',
            )}
          >
            <div className={cn('i-ph:brain h-3.5 w-3.5', isActive && 'animate-pulse')} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span className="text-[13px] font-display font-medium text-cloud-elements-textPrimary">
                {isActive ? 'Thinking...' : 'Reasoning'}
              </span>
              {isActive && startTime ? <LiveDuration startTime={startTime} /> : null}
              {!isActive && durationMs != null ? (
                <span className="rounded-full border border-white/8 bg-white/5 px-1.5 py-0.5 text-[10px] font-data text-cloud-elements-textTertiary">
                  {formatDuration(durationMs)}
                </span>
              ) : null}
            </div>
            {preview && !open && (
              <div className="mt-0.5 truncate text-[11px] text-cloud-elements-textSecondary">{preview}</div>
            )}
          </div>
          <div className={cn('h-3.5 w-3.5 text-cloud-elements-textTertiary', open ? 'i-ph:caret-down' : 'i-ph:caret-right')} />
        </div>
      </button>

      {open && (
        <div className="border-t border-cloud-elements-dividerColor/50 px-3 py-3">
          {part.text ? (
            <AppMarkdown className="text-[13px] leading-6 text-cloud-elements-textSecondary">{part.text}</AppMarkdown>
          ) : (
            <div className="text-[13px] text-cloud-elements-textTertiary">No reasoning text was provided.</div>
          )}
        </div>
      )}
    </div>
  );
}
