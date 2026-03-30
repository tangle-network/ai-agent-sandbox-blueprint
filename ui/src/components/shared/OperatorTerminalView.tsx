import '@xterm/xterm/css/xterm.css';
import { useCallback, useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { useOperatorTerminalSession } from '~/lib/hooks/useOperatorTerminalSession';

interface OperatorTerminalViewProps {
  apiUrl: string;
  resourcePath: string;
  token: string;
  title?: string;
  subtitle?: string;
  initialCwd?: string;
  displayUsername?: string;
  displayPath?: string;
}

const theme = {
  background: '#0c0c0e',
  foreground: '#d4d4d8',
  cursor: '#34d399',
  cursorAccent: '#0c0c0e',
  selectionBackground: '#0f766e55',
  selectionForeground: '#d4d4d8',
  black: '#18181b',
  red: '#ef4444',
  green: '#34d399',
  yellow: '#fbbf24',
  blue: '#60a5fa',
  magenta: '#14b8a6',
  cyan: '#22d3ee',
  white: '#d4d4d8',
  brightBlack: '#52525b',
  brightRed: '#f87171',
  brightGreen: '#6ee7b7',
  brightYellow: '#fde68a',
  brightBlue: '#93c5fd',
  brightMagenta: '#5eead4',
  brightCyan: '#67e8f9',
  brightWhite: '#fafafa',
};

const prompt = '\x1b[38;5;48m$\x1b[0m ';

export function OperatorTerminalView({
  apiUrl,
  resourcePath,
  token,
  title = 'Terminal',
  subtitle = 'Connected through the operator API',
  initialCwd = '',
  displayUsername = '',
  displayPath = '',
}: OperatorTerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const lineBufferRef = useRef('');

  const formatBannerLine = useCallback((value: string) => {
    const normalized = value.trim();
    if (!normalized) return ''.padEnd(37);
    if (normalized.length <= 37) return normalized.padEnd(37);
    return `${normalized.slice(0, 34)}...`;
  }, []);

  const writeBanner = useCallback(() => {
    const term = termRef.current;
    if (!term) return;
    const padTitle = formatBannerLine(title);
    const padSubtitle = formatBannerLine(subtitle);
    const identity = displayUsername && displayPath
      ? formatBannerLine(`${displayUsername} | ${displayPath}`)
      : '';
    term.writeln(`\x1b[38;5;48m\u256d${'─'.repeat(41)}\u256e\x1b[0m`);
    term.writeln(`\x1b[38;5;48m\u2502\x1b[0m  \x1b[1m${padTitle}\x1b[0m\x1b[38;5;48m\u2502\x1b[0m`);
    term.writeln(`\x1b[38;5;48m\u2502\x1b[0m  ${padSubtitle}\x1b[38;5;48m\u2502\x1b[0m`);
    if (identity) {
      term.writeln(`\x1b[38;5;48m\u2502\x1b[0m  ${identity}\x1b[38;5;48m\u2502\x1b[0m`);
    }
    term.writeln(`\x1b[38;5;48m\u2570${'─'.repeat(41)}\u256f\x1b[0m`);
    term.write(prompt);
  }, [displayPath, displayUsername, formatBannerLine, subtitle, title]);

  const writePrompt = useCallback(() => {
    termRef.current?.write(prompt);
  }, []);

  const writePromptOnNewLine = useCallback(() => {
    termRef.current?.write(`\r\n${prompt}`);
  }, []);

  const handleOutput = useCallback((data: string) => {
    if (!data) {
      return;
    }
    termRef.current?.write(data);
    if (!data.endsWith('\n') && !data.endsWith('\r')) {
      termRef.current?.write('\r\n');
    }
  }, []);

  const handleCommandComplete = useCallback(() => {
    termRef.current?.write(prompt);
  }, []);

  const { isConnected, error, sendCommand, reconnect, newSession } = useOperatorTerminalSession({
    apiUrl,
    resourcePath,
    token,
    initialCwd,
    onOutput: handleOutput,
    onCommandComplete: handleCommandComplete,
  });

  const handleNewSession = useCallback(() => {
    const term = termRef.current;
    if (term) {
      lineBufferRef.current = '';
      term.clear();
      writeBanner();
    }
    newSession();
  }, [newSession, writeBanner]);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      theme,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, monospace',
      fontSize: 13,
      lineHeight: 1.4,
      cursorBlink: true,
      cursorStyle: 'bar',
      scrollback: 5000,
      convertEol: true,
      allowProposedApi: true,
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();

    term.loadAddon(fitAddon);
    term.loadAddon(webLinksAddon);
    term.open(containerRef.current);

    requestAnimationFrame(() => {
      fitAddon.fit();
    });

    termRef.current = term;
    writeBanner();

    term.onData((data) => {
      const code = data.charCodeAt(0);

      if (data === '\r') {
        const command = lineBufferRef.current;
        lineBufferRef.current = '';
        term.write('\r\n');

        if (command.trim()) {
          sendCommand(command).catch((err) => {
            term.writeln(`\x1b[31mError: ${err instanceof Error ? err.message : String(err)}\x1b[0m`);
            term.write(prompt);
          });
        } else {
          term.write(prompt);
        }
      } else if (data === '\x7f' || data === '\b') {
        if (lineBufferRef.current.length > 0) {
          lineBufferRef.current = lineBufferRef.current.slice(0, -1);
          term.write('\b \b');
        }
      } else if (data === '\x03') {
        lineBufferRef.current = '';
        term.write('^C');
        writePromptOnNewLine();
      } else if (data === '\x0c') {
        lineBufferRef.current = '';
        term.clear();
        term.write(prompt);
      } else if (code >= 32) {
        lineBufferRef.current += data;
        term.write(data);
      }
    });

    const resizeObserver = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        fitAddon.fit();
      });
    });
    resizeObserver.observe(containerRef.current);

    return () => {
      resizeObserver.disconnect();
      term.dispose();
      termRef.current = null;
    };
  }, [sendCommand, writeBanner, writePromptOnNewLine]);

  return (
    <div className="relative h-full w-full group">
      <div
        ref={containerRef}
        className="h-full w-full overflow-hidden rounded-lg"
        style={{ backgroundColor: theme.background }}
      />

      {/* New Session button — top-right, visible on hover */}
      {isConnected && (
        <button
          onClick={handleNewSession}
          className="absolute top-2 right-2 flex items-center gap-1.5 px-2 py-1 rounded-md text-xs
            bg-neutral-800/80 text-neutral-400 opacity-0 group-hover:opacity-100
            hover:bg-neutral-700 hover:text-neutral-200 transition-all cursor-pointer"
          title="New terminal session"
        >
          <span className="i-ph:plus text-xs" />
          New Session
        </button>
      )}

      {(!isConnected || error) && (
        <div className="absolute inset-0 flex items-center justify-center rounded-lg bg-black/60">
          <div className="text-center px-6">
            {error ? (
              <>
                <p className="mb-3 text-sm text-red-400">{error}</p>
                <button
                  onClick={reconnect}
                  className="cursor-pointer text-sm text-emerald-400 underline hover:text-emerald-300"
                >
                  Retry connection
                </button>
              </>
            ) : (
              <p className="text-sm text-neutral-400">Connecting through operator...</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
