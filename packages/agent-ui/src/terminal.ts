// ---------------------------------------------------------------------------
// @tangle/agent-ui/terminal — xterm.js terminal component (lazy-loadable)
//
// Separated from the main entry to avoid bundling xterm.js (~334KB) for
// consumers who only need chat components.
//
// Usage:
//   const TerminalView = lazy(() =>
//     import('@tangle/agent-ui/terminal').then(m => ({ default: m.TerminalView }))
//   );
// ---------------------------------------------------------------------------

export { default as TerminalView } from './components/terminal/TerminalView';
export type { TerminalViewProps, TerminalTheme } from './components/terminal/TerminalView';
export { DEFAULT_TERMINAL_THEME } from './components/terminal/TerminalView';
