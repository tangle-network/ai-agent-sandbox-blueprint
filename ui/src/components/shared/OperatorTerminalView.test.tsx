import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { OperatorTerminalView } from './OperatorTerminalView';
import { useOperatorTerminalSession } from '~/lib/hooks/useOperatorTerminalSession';

const newSessionMock = vi.fn();
const reconnectMock = vi.fn();
const sendInputMock = vi.fn();

const mockState = vi.hoisted(() => ({
  terminalInstances: [] as Array<{
    cols: number;
    rows: number;
    clear: ReturnType<typeof vi.fn>;
    reset: ReturnType<typeof vi.fn>;
    writeln: ReturnType<typeof vi.fn>;
    write: ReturnType<typeof vi.fn>;
    loadAddon: ReturnType<typeof vi.fn>;
    open: ReturnType<typeof vi.fn>;
    onData: ReturnType<typeof vi.fn>;
    dispose: ReturnType<typeof vi.fn>;
  }>,
  fitAddonInstances: [] as Array<{
    fit: ReturnType<typeof vi.fn>;
  }>,
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    clear = vi.fn();
    reset = vi.fn();
    writeln = vi.fn();
    write = vi.fn();
    loadAddon = vi.fn();
    open = vi.fn();
    onData = vi.fn();
    dispose = vi.fn();

    constructor() {
      mockState.terminalInstances.push(this);
    }
  },
}));

vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn();

    constructor() {
      mockState.fitAddonInstances.push(this);
    }
  },
}));

vi.mock('@xterm/addon-web-links', () => ({
  WebLinksAddon: class {},
}));

vi.mock('~/lib/hooks/useOperatorTerminalSession', () => ({
  useOperatorTerminalSession: vi.fn(),
}));

describe('OperatorTerminalView', () => {
  beforeEach(() => {
    mockState.terminalInstances.length = 0;
    mockState.fitAddonInstances.length = 0;
    newSessionMock.mockReset();
    reconnectMock.mockReset();
    sendInputMock.mockReset();

    vi.mocked(useOperatorTerminalSession).mockReturnValue({
      isConnected: true,
      error: null,
      sessionId: 'term-1',
      sendInput: sendInputMock,
      reconnect: reconnectMock,
      newSession: newSessionMock,
    });

    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    vi.stubGlobal('ResizeObserver', class {
      observe() {}
      disconnect() {}
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('fully resets the terminal before creating a new session', () => {
    render(
      <OperatorTerminalView
        apiUrl="http://operator:9090"
        resourcePath="/api/sandboxes/sb-1"
        token="token-1"
        title="Sandbox Shell"
        subtitle="Secure shell via operator relay"
        initialCwd="/home/agent"
        displayUsername="agent"
        displayPath="/home/agent"
      />,
    );

    const terminal = mockState.terminalInstances[0];
    expect(terminal).toBeDefined();
    expect(mockState.fitAddonInstances[0]?.fit).toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: /new session/i }));

    expect(terminal.reset).toHaveBeenCalledTimes(1);
    expect(terminal.clear).not.toHaveBeenCalled();
    expect(newSessionMock).toHaveBeenCalledTimes(1);
  });

  it('renders the banner with clear shell metadata', () => {
    render(
      <OperatorTerminalView
        apiUrl="http://operator:9090"
        resourcePath="/api/sandboxes/sb-1"
        token="token-1"
        title="Sandbox Shell"
        subtitle="Secure shell via operator relay"
        initialCwd="/home/agent"
        displayUsername="agent"
        displayPath="/home/agent"
      />,
    );

    const terminal = mockState.terminalInstances[0];
    const bannerWrites = terminal.writeln.mock.calls.map(([value]) => String(value));

    expect(bannerWrites.some((line) => line.includes('Sandbox Shell'))).toBe(true);
    expect(bannerWrites.some((line) => line.includes('Secure shell via operator relay'))).toBe(true);
    expect(bannerWrites.some((line) => line.includes('User: agent'))).toBe(true);
    expect(bannerWrites.some((line) => line.includes('Start dir: /home/agent'))).toBe(true);
  });
});
