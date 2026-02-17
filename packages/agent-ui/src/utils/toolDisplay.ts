import type { ToolPart } from '~/types/parts';
import type { ToolDisplayMetadata, DisplayVariant } from '~/types/tool-display';
import type { ToolCategory } from '~/types/run';

// ---------------------------------------------------------------------------
// Tool name normalisation
// ---------------------------------------------------------------------------

const TOOL_NAME_PREFIX = 'tool:';

function normalizeToolName(tool: string | undefined): string {
  const n = tool?.toLowerCase() ?? '';
  return n.startsWith(TOOL_NAME_PREFIX) ? n.slice(TOOL_NAME_PREFIX.length) : n;
}

// ---------------------------------------------------------------------------
// Input helpers
// ---------------------------------------------------------------------------

function extractString(obj: unknown, key: string): string | undefined {
  if (typeof obj === 'object' && obj !== null && key in (obj as Record<string, unknown>)) {
    const val = (obj as Record<string, unknown>)[key];
    if (typeof val === 'string') return val;
  }
  return undefined;
}

function extractCommand(input: unknown): string | undefined {
  if (typeof input === 'string') return input;
  return extractString(input, 'command') ?? extractString(input, 'cmd');
}

function extractFilePath(input: unknown): string | undefined {
  return (
    extractString(input, 'file_path') ??
    extractString(input, 'path') ??
    extractString(input, 'filePath')
  );
}

function cleanPath(path: string): string {
  const parts = path.split('/');
  return parts.length > 3 ? '.../' + parts.slice(-2).join('/') : path;
}

// ---------------------------------------------------------------------------
// Category classification
// ---------------------------------------------------------------------------

export function getToolCategory(toolName: string): ToolCategory {
  const name = normalizeToolName(toolName);
  switch (name) {
    case 'bash':
    case 'shell':
    case 'command':
    case 'execute':
      return 'command';
    case 'write':
    case 'write_file':
    case 'create_file':
      return 'write';
    case 'read':
    case 'read_file':
    case 'cat':
      return 'read';
    case 'grep':
    case 'search':
    case 'rg':
      return 'search';
    case 'edit':
    case 'patch':
    case 'sed':
      return 'edit';
    case 'glob':
    case 'find':
    case 'ls':
      return 'search';
    case 'web_search':
    case 'web_fetch':
    case 'fetch':
      return 'web';
    case 'task':
    case 'agent':
    case 'spawn':
      return 'task';
    case 'todo':
    case 'todo_write':
      return 'todo';
    default:
      return 'other';
  }
}

/** Icon class for a tool category. */
export const TOOL_CATEGORY_ICONS: Record<ToolCategory, string> = {
  command: 'i-ph:terminal-window',
  write: 'i-ph:note-pencil',
  read: 'i-ph:file-magnifying-glass',
  search: 'i-ph:magnifying-glass',
  edit: 'i-ph:pencil-simple-line',
  task: 'i-ph:robot',
  web: 'i-ph:globe-hemisphere-west',
  todo: 'i-ph:clipboard-text',
  other: 'i-ph:gear',
};

// ---------------------------------------------------------------------------
// Main metadata resolver
// ---------------------------------------------------------------------------

export function getToolDisplayMetadata(part: ToolPart): ToolDisplayMetadata {
  const name = normalizeToolName(part.tool);
  const input = part.state.status !== 'pending' ? part.state.input : undefined;
  const filePath = extractFilePath(input);
  const command = extractCommand(input);

  switch (name) {
    case 'bash':
    case 'shell':
    case 'command':
    case 'execute':
      return {
        title: 'Run command',
        description: command ? truncateCommand(command) : undefined,
        iconClass: 'i-ph:terminal-window',
        displayVariant: 'command',
        commandSnippet: command,
      };

    case 'write':
    case 'write_file':
    case 'create_file':
      return {
        title: filePath ? `Write ${cleanPath(filePath)}` : 'Write file',
        description: filePath,
        iconClass: 'i-ph:note-pencil',
        displayVariant: 'write-file',
        targetPath: filePath,
      };

    case 'edit':
    case 'patch':
      return {
        title: filePath ? `Edit ${cleanPath(filePath)}` : 'Edit file',
        description: filePath,
        iconClass: 'i-ph:pencil-simple-line',
        hasDiffOutput: true,
        diffFilePath: filePath,
        targetPath: filePath,
      };

    case 'read':
    case 'read_file':
    case 'cat':
      return {
        title: filePath ? `Read ${cleanPath(filePath)}` : 'Read file',
        description: filePath,
        iconClass: 'i-ph:file-magnifying-glass',
        targetPath: filePath,
      };

    case 'grep':
    case 'search':
    case 'rg': {
      const pattern = extractString(input, 'pattern');
      return {
        title: pattern ? `Search: ${pattern}` : 'Search',
        description: pattern,
        iconClass: 'i-ph:magnifying-glass',
        displayVariant: 'grep',
      };
    }

    case 'glob':
    case 'find':
    case 'ls': {
      const pattern = extractString(input, 'pattern');
      return {
        title: pattern ? `Find: ${pattern}` : 'Find files',
        description: pattern,
        iconClass: 'i-ph:list-bullets',
        displayVariant: 'glob',
      };
    }

    case 'web_search':
    case 'web_fetch':
    case 'fetch': {
      const query = extractString(input, 'query') ?? extractString(input, 'url');
      return {
        title: query ? `Web: ${truncateCommand(query)}` : 'Web search',
        description: query,
        iconClass: 'i-ph:globe-hemisphere-west',
        displayVariant: 'web-search',
      };
    }

    case 'task':
    case 'agent':
    case 'spawn': {
      const desc = extractString(input, 'description') ?? extractString(input, 'prompt');
      return {
        title: desc ? `Task: ${truncateCommand(desc)}` : 'Agent task',
        description: desc,
        iconClass: 'i-ph:robot',
      };
    }

    default:
      return {
        title: part.tool || 'Tool',
        iconClass: 'i-ph:gear',
      };
  }
}

function truncateCommand(cmd: string): string {
  const first = cmd.split('\n')[0];
  return first.length > 60 ? first.slice(0, 57) + '...' : first;
}

/** Extract error text from a tool part, if any. */
export function getToolErrorText(part: ToolPart, fallback?: string): string | undefined {
  if (part.state.status !== 'error') return undefined;
  return part.state.error || fallback;
}
