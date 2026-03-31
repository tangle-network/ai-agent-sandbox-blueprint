import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import InstanceDetail from './instances.$id';

const {
  instancesRef,
  mockOperatorApiCall,
  mockGetOperatorToken,
  mockRefreshInstances,
  mockUpdateInstanceStatus,
  operatorAuthState,
} = vi.hoisted(() => ({
  instancesRef: { current: [] as Array<Record<string, unknown>> },
  mockOperatorApiCall: vi.fn(),
  mockGetOperatorToken: vi.fn(),
  mockRefreshInstances: vi.fn(),
  mockUpdateInstanceStatus: vi.fn(),
  operatorAuthState: {
    isAuthenticated: false,
    isAuthenticating: false,
    error: null as string | null,
    cachedToken: null as string | null,
  },
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
  useParams: () => ({ id: 'instance-1' }),
}));

vi.mock('@nanostores/react', () => ({
  useStore: () => instancesRef.current,
}));

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  AnimatedPage: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Card: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardContent: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardHeader: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardTitle: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardDescription: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
  Textarea: (props: React.TextareaHTMLAttributes<HTMLTextAreaElement>) => <textarea {...props} />,
  Dialog: ({ open, children }: { open: boolean; children: React.ReactNode }) => (open ? <div>{children}</div> : null),
  DialogContent: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  DialogHeader: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  DialogTitle: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  DialogDescription: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
}));

vi.mock('@tangle-network/blueprint-ui', () => ({
  getBlueprint: () => ({ name: 'AI Agent Instance' }),
  cn: (...values: Array<string | false | null | undefined>) => values.filter(Boolean).join(' '),
}));

vi.mock('~/lib/utils/truncate-address', () => ({
  truncateAddress: (value: string) => {
    if (!value || value.length <= 12) return value;
    return `${value.slice(0, 6)}...${value.slice(-4)}`;
  },
}));

vi.mock('~/lib/stores/instances', () => ({
  instanceListStore: {},
  getInstance: (key: string) =>
    instancesRef.current.find((instance) => instance.id === key || instance.sandboxId === key),
  updateInstanceStatus: mockUpdateInstanceStatus,
}));

vi.mock('~/lib/hooks/useOperatorAuth', () => ({
  useOperatorAuth: () => ({
    getToken: mockGetOperatorToken,
    getCachedToken: () => operatorAuthState.cachedToken,
    isAuthenticated: operatorAuthState.isAuthenticated,
    isAuthenticating: operatorAuthState.isAuthenticating,
    error: operatorAuthState.error,
  }),
}));

vi.mock('~/lib/hooks/useOperatorApiCall', () => ({
  useOperatorApiCall: () => mockOperatorApiCall,
}));

vi.mock('~/lib/hooks/useInstanceHydration', () => ({
  useInstanceHydration: () => ({
    refresh: mockRefreshInstances,
    isHydrating: false,
    authRequired: false,
    lastError: null,
  }),
}));

vi.mock('~/lib/hooks/useProvisionWatcher', () => ({
  useInstanceProvisionWatcher: () => null,
}));

vi.mock('~/lib/hooks/useExposedPorts', () => ({
  useExposedPorts: () => [],
}));

vi.mock('~/lib/hooks/useTeeAttestation', () => ({
  useTeeAttestation: () => ({
    attestation: null,
    busy: false,
    error: null,
    fetchAttestation: vi.fn(),
  }),
}));

vi.mock('~/lib/api/sandboxClient', () => ({
  createProxiedInstanceClient: () => ({}),
}));

vi.mock('wagmi', () => ({
  useAccount: () => ({ address: '0x123400000000000000000000000000000000abcd' }),
}));

vi.mock('~/components/shared/SessionSidebar', () => ({
  SessionSidebar: () => <div>Session Sidebar</div>,
}));

vi.mock('~/components/shared/ResourceIdentity', () => ({
  ResourceIdentity: ({ name, statusLabel }: { name: string; statusLabel?: string }) => (
    <div>{name} {statusLabel}</div>
  ),
}));

vi.mock('~/components/shared/LabeledValueRow', () => ({
  LabeledValueRow: ({ label, value }: { label: string; value: string }) => (
    <div>{label}: {value}</div>
  ),
}));

vi.mock('~/components/shared/ExposedPortsCard', () => ({
  ExposedPortsCard: () => <div>Exposed Ports</div>,
}));

vi.mock('~/components/shared/TeeAttestationCard', () => ({
  TeeAttestationCard: () => <div>Attestation</div>,
}));

vi.mock('~/components/shared/ResourceTabs', () => ({
  ResourceTabs: ({
    tabs,
    value,
    onValueChange,
  }: {
    tabs: Array<{ key: string; label: string; hidden?: boolean }>;
    value: string;
    onValueChange: (value: string) => void;
  }) => (
    <div>
      {tabs.filter((tab) => !tab.hidden).map((tab) => (
        <button
          key={tab.key}
          type="button"
          aria-pressed={tab.key === value}
          onClick={() => onValueChange(tab.key)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  ),
}));

vi.mock('~/components/shared/OperatorTerminalView', () => ({
  OperatorTerminalView: ({
    initialCwd,
    displayUsername,
    displayPath,
  }: {
    initialCwd?: string;
    displayUsername?: string;
    displayPath?: string;
  }) => (
    <div data-testid="operator-terminal">
      Operator Terminal
      {displayUsername ? ` ${displayUsername}` : ''}
      {displayPath ? ` ${displayPath}` : ''}
      {initialCwd ? ` ${initialCwd}` : ''}
    </div>
  ),
}));

vi.mock('~/components/shared/OnChainVerificationCard', () => ({
  OnChainVerificationCard: () => <div data-testid="on-chain-verification">On-Chain Verification</div>,
}));

vi.mock('~/components/shared/ConfirmDialog', () => ({
  ConfirmDialog: ({
    open,
    title,
    description,
    confirmLabel,
    onConfirm,
  }: {
    open: boolean;
    title: string;
    description: string;
    confirmLabel?: string;
    onConfirm: () => void;
  }) => (
    open ? (
      <div>
        <div>{title}</div>
        <div>{description}</div>
        <button type="button" onClick={onConfirm}>{confirmLabel ?? 'Confirm'}</button>
      </div>
    ) : null
  ),
}));

function makeInstance(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    id: 'instance-1',
    sandboxId: 'sandbox-instance-1',
    name: 'Worker Instance',
    image: 'tangle-sidecar:local',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: 'ai-agent-instance-blueprint',
    serviceId: '7',
    sidecarUrl: 'http://127.0.0.1:8080',
    teeEnabled: false,
    agentIdentifier: '',
    status: 'running',
    ...overrides,
  };
}

function renderSubject() {
  return render(<InstanceDetail />);
}

describe('InstanceDetail overview card', () => {
  beforeEach(() => {
    instancesRef.current = [makeInstance({
      operator: '0x123400000000000000000000000000000000abcd',
      txHash: '0xabc1234567890def1234567890abcdef1234567890abcdef1234567890abcdef',
    })];
    mockOperatorApiCall.mockReset();
    mockGetOperatorToken.mockReset();
    mockRefreshInstances.mockReset();
    mockUpdateInstanceStatus.mockReset();
    operatorAuthState.isAuthenticated = false;
    operatorAuthState.isAuthenticating = false;
    operatorAuthState.error = null;
    operatorAuthState.cachedToken = null;
    mockOperatorApiCall.mockResolvedValue(new Response('{}', { status: 200 }));
  });

  it('renders sandbox-matching runtime details', () => {
    renderSubject();

    expect(screen.getByText('Runtime Details')).toBeInTheDocument();
    expect(screen.getByText('Operator: 0x1234...abcd')).toBeInTheDocument();
    expect(screen.getByText('TX Hash: 0xabc1...cdef')).toBeInTheDocument();
    expect(screen.queryByText('Connection')).not.toBeInTheDocument();
    expect(screen.queryByText('Access: Operator API')).not.toBeInTheDocument();
    expect(screen.queryByText('Authenticated: No')).not.toBeInTheDocument();
  });

  it('shows Unknown when operator is unavailable', () => {
    instancesRef.current = [makeInstance({ operator: undefined, txHash: undefined })];

    renderSubject();

    expect(screen.getByText('Operator: Unknown')).toBeInTheDocument();
    expect(screen.queryByText(/TX Hash:/)).not.toBeInTheDocument();
  });

  it('shows terminal when terminal tab is selected and hides it on switch', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';

    renderSubject();

    expect(screen.queryByTestId('operator-terminal')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Terminal' }));
    await screen.findByTestId('operator-terminal');

    fireEvent.click(screen.getByRole('button', { name: 'Overview' }));
    expect(screen.queryByTestId('operator-terminal')).not.toBeInTheDocument();
  });
});

describe('InstanceDetail secrets tab', () => {
  beforeEach(() => {
    instancesRef.current = [makeInstance()];
    mockOperatorApiCall.mockReset();
    mockGetOperatorToken.mockReset();
    mockRefreshInstances.mockReset();
    mockUpdateInstanceStatus.mockReset();
    operatorAuthState.isAuthenticated = false;
    operatorAuthState.isAuthenticating = false;
    operatorAuthState.error = null;
    operatorAuthState.cachedToken = null;
    mockGetOperatorToken.mockResolvedValue('operator-token');
    mockRefreshInstances.mockResolvedValue(true);
    mockOperatorApiCall.mockResolvedValue(new Response('{}', { status: 200 }));
  });

  it('shows the secrets tab for non-TEE instances', () => {
    renderSubject();

    expect(screen.getByRole('button', { name: 'Secrets' })).toBeInTheDocument();
  });

  it('hides the secrets tab for TEE instances', () => {
    instancesRef.current = [makeInstance({ teeEnabled: true })];

    renderSubject();

    expect(screen.queryByRole('button', { name: 'Secrets' })).not.toBeInTheDocument();
  });

  it('prompts for operator auth on the secrets tab', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Secrets' }));

    expect(await screen.findByText(/Authenticate with the operator to manage instance secrets/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Authenticate' })).toBeInTheDocument();
  });

  it('submits injected secrets and refreshes instance state', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';

    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Secrets' }));
    // Wait for initial secrets fetch to complete
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Inject Secrets' })).toBeEnabled();
    });
    fireEvent.change(screen.getByLabelText('Secrets (JSON object)'), {
      target: { value: '{"API_KEY":"sk-test"}' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Inject Secrets' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('secrets', { env_json: { API_KEY: 'sk-test' } });
    });
    expect(mockRefreshInstances).toHaveBeenCalledWith({ interactive: true });
    expect(await screen.findByText('Secrets injected')).toBeInTheDocument();
  });

  it('blocks invalid secrets JSON on the client', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';

    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Secrets' }));
    // Wait for initial secrets fetch to complete
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Inject Secrets' })).toBeEnabled();
    });
    mockOperatorApiCall.mockClear();
    fireEvent.change(screen.getByLabelText('Secrets (JSON object)'), {
      target: { value: 'nope' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Inject Secrets' }));

    expect(await screen.findByText(/Unexpected token|not valid JSON/i)).toBeInTheDocument();
    expect(mockOperatorApiCall).not.toHaveBeenCalled();
  });

  it('wipes secrets after confirmation and refreshes instance state', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';

    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Secrets' }));
    // Wait for initial secrets fetch to complete
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Wipe All Secrets' })).toBeEnabled();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Wipe All Secrets' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Wipe' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('secrets', undefined, { method: 'DELETE' });
    });
    expect(mockRefreshInstances).toHaveBeenCalledWith({ interactive: true });
    expect(await screen.findByText('Secrets wiped')).toBeInTheDocument();
  });
});

describe('InstanceDetail SSH tab', () => {
  beforeEach(() => {
    instancesRef.current = [makeInstance({ sshPort: 2222 })];
    mockOperatorApiCall.mockReset();
    mockGetOperatorToken.mockReset();
    mockRefreshInstances.mockReset();
    mockUpdateInstanceStatus.mockReset();
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.isAuthenticating = false;
    operatorAuthState.error = null;
    operatorAuthState.cachedToken = 'operator-token';
    mockGetOperatorToken.mockResolvedValue('operator-token');
    mockOperatorApiCall.mockResolvedValue(
      new Response(JSON.stringify({ username: 'sidecar' }), { status: 200 }),
    );
  });

  it('hides SSH tab when sshPort is absent', () => {
    instancesRef.current = [makeInstance()];

    renderSubject();

    expect(screen.queryByRole('button', { name: 'SSH' })).not.toBeInTheDocument();
  });

  it('shows SSH tab when sshPort is present', () => {
    renderSubject();

    expect(screen.getByRole('button', { name: 'SSH' })).toBeInTheDocument();
  });

  it('auto-detects the SSH username when SSH tab opens', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'SSH' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('ssh/user', undefined, { method: 'GET' });
    });
    expect(await screen.findByText(/Detected sandbox user: sidecar/)).toBeInTheDocument();
  });

  it('starts the terminal in the detected SSH user home directory', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Terminal' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('ssh/user', undefined, { method: 'GET' });
    });
    expect(await screen.findByText(/Operator Terminal sidecar \/home\/sidecar \/home\/sidecar/)).toBeInTheDocument();
  });

  it('stores backend-returned SSH username after provision', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'SSH' }));

    // Wait for detection to complete first
    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('ssh/user', undefined, { method: 'GET' });
    });

    // Reset mock to track provision call
    mockOperatorApiCall.mockResolvedValue(
      new Response(JSON.stringify({ username: 'sidecar' }), { status: 200 }),
    );

    const keyInput = screen.getByLabelText('SSH public key');
    fireEvent.change(keyInput, { target: { value: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add Key' }));

    await waitFor(() => {
      // Username is auto-detected as 'sidecar', so it's included in the provision payload
      expect(mockOperatorApiCall).toHaveBeenCalledWith('ssh', {
        public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest',
        username: 'sidecar',
      });
    });
    expect(await screen.findByText('SSH key provisioned')).toBeInTheDocument();
  });
});
