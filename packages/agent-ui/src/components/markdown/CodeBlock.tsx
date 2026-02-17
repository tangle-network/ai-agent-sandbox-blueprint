import { memo, useCallback, useState, type HTMLAttributes, type ReactNode } from 'react';
import { cn } from '~/utils/cn';

export interface CodeBlockProps extends HTMLAttributes<HTMLDivElement> {
  code: string;
  language?: string;
  children?: ReactNode;
}

/**
 * Syntax-highlighted code block with copy button.
 *
 * Falls back to plain `<pre><code>` — syntax highlighting can be provided
 * by the consuming app via CSS (e.g. Shiki themes applied externally).
 */
export const CodeBlock = memo(({ code, language, className, children, ...props }: CodeBlockProps) => {
  return (
    <div className={cn('relative overflow-hidden rounded-lg', className)} {...props}>
      <div className="relative">
        <pre className="m-0 p-4 overflow-x-auto text-sm font-mono bg-neutral-900 text-neutral-100 dark:bg-neutral-950">
          <code className={language ? `language-${language}` : undefined}>{code}</code>
        </pre>
        {children && (
          <div className="absolute top-2 right-2 flex items-center gap-2">{children}</div>
        )}
      </div>
    </div>
  );
});
CodeBlock.displayName = 'CodeBlock';

/** Copy-to-clipboard button for use inside CodeBlock. */
export const CopyButton = memo(({ text }: { text: string }) => {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch { /* noop */ }
  }, [text]);

  return (
    <button
      onClick={handleCopy}
      className="flex items-center justify-center w-7 h-7 rounded-md bg-white/10 hover:bg-white/20 transition-colors"
      title="Copy to clipboard"
    >
      <div className={cn('w-3.5 h-3.5', copied ? 'i-ph:check text-green-400' : 'i-ph:copy text-neutral-400')} />
    </button>
  );
});
CopyButton.displayName = 'CopyButton';
