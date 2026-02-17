import { memo } from 'react';
import { cn } from '~/utils/cn';
import type { ToolPart } from '~/types/parts';
import { CodeBlock, CopyButton } from '../markdown/CodeBlock';

export interface WriteFilePreviewProps {
  part: ToolPart;
}

function extractWriteContent(input: unknown): { path: string; content: string } | null {
  if (!input || typeof input !== 'object') return null;
  const obj = input as Record<string, unknown>;
  const path = String(obj.file_path ?? obj.path ?? obj.filePath ?? 'unknown');
  const content = String(obj.content ?? obj.contents ?? obj.data ?? '');
  return { path, content };
}

function getLanguageFromPath(path: string): string | undefined {
  const ext = path.split('.').pop()?.toLowerCase();
  const map: Record<string, string> = {
    ts: 'typescript', tsx: 'tsx', js: 'javascript', jsx: 'jsx',
    rs: 'rust', py: 'python', go: 'go', rb: 'ruby',
    json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml',
    md: 'markdown', css: 'css', scss: 'scss', html: 'html',
    sh: 'bash', bash: 'bash', zsh: 'bash', sql: 'sql',
    sol: 'solidity', proto: 'protobuf',
  };
  return ext ? map[ext] : undefined;
}

/**
 * Preview for file write/create operations.
 * Shows file path, line count, and the written content.
 */
export const WriteFilePreview = memo(({ part }: WriteFilePreviewProps) => {
  const write = extractWriteContent(part.state.input);
  if (!write) return null;

  const lineCount = write.content.split('\n').length;
  const language = getLanguageFromPath(write.path);

  return (
    <div className="rounded-lg overflow-hidden border border-neutral-700/50">
      <div className="flex items-center gap-2 px-3 py-2 bg-neutral-800/80">
        <div className="i-ph:note-pencil w-3.5 h-3.5 text-neutral-400 shrink-0" />
        <span className="text-xs font-mono text-neutral-300 truncate flex-1">{write.path}</span>
        <span className="text-xs text-neutral-500">
          +{lineCount} line{lineCount !== 1 ? 's' : ''}
        </span>
      </div>
      <CodeBlock code={write.content} language={language} className="rounded-none">
        <CopyButton text={write.content} />
      </CodeBlock>
    </div>
  );
});
WriteFilePreview.displayName = 'WriteFilePreview';
