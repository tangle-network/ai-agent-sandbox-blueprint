// ---------------------------------------------------------------------------
// @tangle/agent-ui — shared components for agentic chat UIs
// ---------------------------------------------------------------------------

// Types
export type { SessionMessage } from './types/message';
export type {
  TextPart,
  ToolPart,
  ToolState,
  ToolStatus,
  ToolTime,
  ReasoningPart,
  SessionPart,
} from './types/parts';
export type {
  Run,
  RunStats,
  FinalTextPart,
  ToolCategory,
  GroupedMessage,
  MessageRun,
  MessageUser,
} from './types/run';
export type { ToolDisplayMetadata, DisplayVariant } from './types/tool-display';
export type { AgentBranding } from './types/branding';

// Stores
export { messagesAtom, partMapAtom, isStreamingAtom, addMessage, addParts, updatePart, clearChat } from './stores/chatStore';
export { sessionAtom, connectSession, disconnectSession } from './stores/sessionStore';
export type { ChatSession } from './stores/sessionStore';

// Hooks
export { useRunGroups } from './hooks/useRunGroups';
export type { UseRunGroupsOptions } from './hooks/useRunGroups';
export { useRunCollapseState } from './hooks/useRunCollapseState';
export { useAutoScroll } from './hooks/useAutoScroll';

// Utils
export { cn } from './utils/cn';
export { formatDuration, truncateText } from './utils/format';
export {
  getToolDisplayMetadata,
  getToolCategory,
  getToolErrorText,
  TOOL_CATEGORY_ICONS,
} from './utils/toolDisplay';

// Components — Markdown
export { CodeBlock, CopyButton } from './components/markdown/CodeBlock';
export type { CodeBlockProps } from './components/markdown/CodeBlock';
export { Markdown } from './components/markdown/Markdown';
export type { MarkdownProps } from './components/markdown/Markdown';

// Components — Tool previews
export { CommandPreview } from './components/tool-previews/CommandPreview';
export type { CommandPreviewProps } from './components/tool-previews/CommandPreview';
export { WriteFilePreview } from './components/tool-previews/WriteFilePreview';
export type { WriteFilePreviewProps } from './components/tool-previews/WriteFilePreview';

// Components — Run
export { RunGroup } from './components/run/RunGroup';
export type { RunGroupProps } from './components/run/RunGroup';
export { InlineToolItem } from './components/run/InlineToolItem';
export type { InlineToolItemProps } from './components/run/InlineToolItem';
export { InlineThinkingItem } from './components/run/InlineThinkingItem';
export type { InlineThinkingItemProps } from './components/run/InlineThinkingItem';
export { ExpandedToolDetail } from './components/run/ExpandedToolDetail';
export type { ExpandedToolDetailProps } from './components/run/ExpandedToolDetail';

// Components — Chat
export { ChatContainer } from './components/chat/ChatContainer';
export type { ChatContainerProps } from './components/chat/ChatContainer';
export { MessageList } from './components/chat/MessageList';
export type { MessageListProps } from './components/chat/MessageList';
export { UserMessage } from './components/chat/UserMessage';
export type { UserMessageProps } from './components/chat/UserMessage';
