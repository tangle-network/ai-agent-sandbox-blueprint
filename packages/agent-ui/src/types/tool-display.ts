/**
 * Variant-specific rendering instructions for tool output.
 * Maps directly to specialised preview components.
 */
export type DisplayVariant =
  | 'command'
  | 'write-file'
  | 'question'
  | 'web-search'
  | 'grep'
  | 'glob'
  | 'default';

/**
 * Custom renderer for tool details. Return a ReactNode to override the
 * default ExpandedToolDetail, or null to fall back to the built-in renderer.
 */
export type CustomToolRenderer = (part: import('./parts').ToolPart) => import('react').ReactNode | null;

/**
 * Visual metadata for a tool invocation — computed from the tool name,
 * input, and output by `getToolDisplayMetadata()`.
 */
export interface ToolDisplayMetadata {
  title: string;
  description?: string;
  /** UnoCSS icon class, e.g. `'i-ph:terminal-window'` */
  iconClass: string;
  inputTitle?: string;
  outputTitle?: string;
  inputLanguage?: string;
  outputLanguage?: string;
  /** Whether this tool produces a unified diff that can be rendered. */
  hasDiffOutput?: boolean;
  /** File path relevant to the tool action. */
  diffFilePath?: string;
  displayVariant?: DisplayVariant;
  /** Extracted shell command snippet (for command display variant). */
  commandSnippet?: string;
  /** Target file path (for write / edit tools). */
  targetPath?: string;
}
