import { memo, type ComponentProps } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeSanitize from 'rehype-sanitize';
import { CodeBlock, CopyButton } from './CodeBlock';
import { cn } from '~/utils/cn';

export interface MarkdownProps {
  children: string;
  className?: string;
}

/**
 * Renders Markdown content with GFM support, XSS sanitisation, and
 * custom code block rendering via our CodeBlock component.
 */
export const Markdown = memo(({ children, className }: MarkdownProps) => {
  return (
    <div className={cn('prose prose-sm dark:prose-invert max-w-none', className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSanitize]}
        components={{
          pre({ children: preChildren }) {
            return <>{preChildren}</>;
          },
          code({ className: codeClass, children: codeChildren, ...rest }) {
            const match = /language-(\w+)/.exec(codeClass || '');
            const language = match?.[1];
            const code = String(codeChildren).replace(/\n$/, '');

            // Inline code (no language fence)
            if (!language && !code.includes('\n')) {
              return (
                <code className={cn('px-1.5 py-0.5 rounded bg-neutral-800/50 text-sm font-mono', codeClass)} {...rest}>
                  {codeChildren}
                </code>
              );
            }

            return (
              <CodeBlock code={code} language={language}>
                <CopyButton text={code} />
              </CodeBlock>
            );
          },
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
});
Markdown.displayName = 'Markdown';
