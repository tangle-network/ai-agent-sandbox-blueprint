import { useState, useCallback } from 'react';
import { cn } from '@tangle/blueprint-ui';

interface CopyButtonProps {
  value: string;
  className?: string;
}

export function CopyButton({ value, className }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // fallback
      const el = document.createElement('textarea');
      el.value = value;
      document.body.appendChild(el);
      el.select();
      document.execCommand('copy');
      document.body.removeChild(el);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  }, [value]);

  return (
    <button
      type="button"
      onClick={handleCopy}
      className={cn(
        'shrink-0 p-0.5 rounded transition-colors',
        copied
          ? 'text-teal-400'
          : 'text-cloud-elements-textTertiary opacity-0 group-hover:opacity-100 hover:text-cloud-elements-textSecondary',
        className,
      )}
      title={copied ? 'Copied!' : 'Copy to clipboard'}
    >
      <div className={cn('text-xs', copied ? 'i-ph:check-bold' : 'i-ph:copy')} />
    </button>
  );
}
