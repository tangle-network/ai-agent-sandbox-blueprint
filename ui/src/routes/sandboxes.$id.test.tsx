import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SandboxDetail from './sandboxes.$id';

const {
  sandboxesRef,
  mockNavigate,
  mockOperatorApiCall,
  mockGetOperatorToken,
  mockRefreshSandboxState,
  mockUpdateSandboxStatus,
  mockToastSuccess,
  mockToastError,
  mockSubmitJob,
  operatorAuthState,
} = vi.hoisted(() => ({
  sandboxesRef: { current: [] as Array<Record<string, unknown>> },
  mockNavigate: vi.fn(),
  mockOperatorApiCall: vi.fn(),
  mockGetOperatorToken: vi.fn(),
  mockRefreshSandboxState: vi.fn(),
  mockUpdateSandboxStatus: vi.fn(),
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
  mockSubmitJob: vi.fn(),
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
  useNavigate: () => mockNavigate,
  useParams: () => ({ id: 'sandbox-1' }),
}));

vi.mock('@nanostores/react', () => ({
  useStore: () => sandboxesRef.current,
}));

vi.mock('sonner', () => ({
  toast: {
    success: mockToastSuccess,
    error: mockToastError,
  },
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
  useSubmitJob: () => ({
    submitJob: mockSubmitJob,
    status: 'idle',
    txHash: undefined,
  }),
  encodeJobArgs: vi.fn(),
  getJobById: vi.fn(),
  cn: (...values: Array<string | false | null | undefined>) => values.filter(Boolean).join(' '),
}));

vi.mock('~/lib/stores/sandboxes', () => ({
  sandboxListStore: {},
  findSandboxByKey: (sandboxes: Array<Record<string, unknown>>, key: string) =>
    sandboxes.find((sandbox) => sandbox.localId === key || sandbox.sandboxId === key),
  getSandboxRouteKey: (sandbox: { localId: string; sandboxId?: string }) => sandbox.sandboxId ?? sandbox.localId,
  updateSandboxStatus: mockUpdateSandboxStatus,
}));

vi.mock('~/lib/hooks/useSandboxReads', () => ({
  useSandboxActive: () => ({ data: true }),
  useSandboxOperator: () => ({ data: '0x123400000000000000000000000000000000abcd' }),
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

vi.mock('~/lib/hooks/useSandboxHydration', () => ({
  useSandboxHydration: () => ({
    refresh: mockRefreshSandboxState,
    isHydrating: false,
    authRequired: false,
    lastError: null,
  }),
}));

vi.mock('wagmi', () => ({
  useAccount: () => ({ address: '0x123400000000000000000000000000000000abcd' }),
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

vi.mock('~/components/shared/SessionSidebar', () => ({
  SessionSidebar: () => <div>Session Sidebar</div>,
}));

vi.mock('~/components/shared/ResourceIdentity', () => ({
  ResourceIdentity: ({ name, status }: { name: string; status: string }) => (
    <div>{name} ({status})</div>
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
    tabs: Array<{ key: string; label: string; disabled?: boolean; hidden?: boolean }>;
    value: string;
    onValueChange: (value: string) => void;
  }) => (
    <div>
      {tabs.filter((tab) => !tab.hidden).map((tab) => (
        <button
          key={tab.key}
          type="button"
          disabled={tab.disabled}
          aria-pressed={tab.key === value}
          onClick={() => onValueChange(tab.key)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  ),
}));

vi.mock('~/components/shared/ProvisionProgress', () => ({
  ProvisionProgress: () => <div>Provision Progress</div>,
}));

vi.mock('~/components/shared/JobPriceBadge', () => ({
  JobPriceBadge: () => <span>Price</span>,
}));

vi.mock('~/components/shared/OperatorTerminalView', () => ({
  OperatorTerminalView: () => <div>Operator Terminal</div>,
}));

function makeSandbox(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    localId: 'draft:sandbox-1',
    sandboxId: 'sandbox-1',
    name: 'Snapshot Sandbox',
    image: 'tangle-sidecar:local',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: 'ai-agent-sandbox-blueprint',
    serviceId: '1',
    operator: '0x123400000000000000000000000000000000abcd',
    sidecarUrl: 'http://127.0.0.1:8080',
    teeEnabled: false,
    agentIdentifier: '',
    status: 'running',
    ...overrides,
  };
}

function renderSubject() {
  return render(<SandboxDetail />);
}

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status });
}

describe('SandboxDetail snapshot flow', () => {
  beforeEach(() => {
    sandboxesRef.current = [makeSandbox()];
    mockNavigate.mockReset();
    mockOperatorApiCall.mockReset();
    mockGetOperatorToken.mockReset();
    mockRefreshSandboxState.mockReset();
    mockUpdateSandboxStatus.mockReset();
    mockToastSuccess.mockReset();
    mockToastError.mockReset();
    mockSubmitJob.mockReset();
    operatorAuthState.isAuthenticated = false;
    operatorAuthState.isAuthenticating = false;
    operatorAuthState.error = null;
    operatorAuthState.cachedToken = null;
    mockGetOperatorToken.mockResolvedValue('operator-token');
    mockRefreshSandboxState.mockResolvedValue(true);
    mockOperatorApiCall.mockResolvedValue(new Response('{}', { status: 200 }));
  });

  it('opens the snapshot modal from the sandbox detail actions', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Snapshot' }));

    expect(await screen.findByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i)).toBeInTheDocument();
    expect(screen.getByText('Destination URL')).toBeInTheDocument();
  });

  it('blocks obviously invalid destinations on the client', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Snapshot' }));
    fireEvent.change(await screen.findByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i), {
      target: { value: 'foo' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Take Snapshot' }));

    expect(await screen.findByText('Destination must start with https:// or s3://')).toBeInTheDocument();
    expect(mockOperatorApiCall).not.toHaveBeenCalled();
  });

  it('submits the snapshot request with default checkbox values', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Snapshot' }));
    fireEvent.change(await screen.findByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i), {
      target: { value: 's3://bucket/snap.tar.gz' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Take Snapshot' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('snapshot', {
        destination: 's3://bucket/snap.tar.gz',
        include_workspace: true,
        include_state: true,
      });
    });
  });

  it('rehydrates operator state after resume instead of forcing local running status', async () => {
    sandboxesRef.current = [makeSandbox({ status: 'stopped' })];
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('resume');
      expect(mockRefreshSandboxState).toHaveBeenCalledWith({ interactive: true });
    });
    expect(mockUpdateSandboxStatus).not.toHaveBeenCalledWith('sandbox-1', 'running');
  });

  it('respects checkbox state changes in the snapshot payload', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Snapshot' }));
    fireEvent.change(await screen.findByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i), {
      target: { value: 'https://93.184.216.34/snap.tar.gz' },
    });
    fireEvent.click(screen.getByLabelText('Workspace files'));
    fireEvent.click(screen.getByRole('button', { name: 'Take Snapshot' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('snapshot', {
        destination: 'https://93.184.216.34/snap.tar.gz',
        include_workspace: false,
        include_state: true,
      });
    });
  });

  it('hides raw sidecar access details from the overview', () => {
    renderSubject();

    expect(screen.queryByText('Sidecar: http://127.0.0.1:8080')).not.toBeInTheDocument();
    expect(screen.getByText('Access: Operator API')).toBeInTheDocument();
  });

  it('prompts for operator auth on the terminal tab', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Terminal' }));

    expect(await screen.findByText(/Authenticate with the operator to access the sandbox terminal/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Connect Terminal' })).toBeInTheDocument();
  });

  it('renders operator-backed terminal after authentication', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';

    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Terminal' }));

    expect(await screen.findByText('Operator Terminal')).toBeInTheDocument();
  });

  it('renders provision progress when a creating sandbox has callId 0', () => {
    sandboxesRef.current = [makeSandbox({ status: 'creating', callId: 0 })];

    renderSubject();

    expect(screen.getByText('Provision Progress')).toBeInTheDocument();
  });

  it('surfaces backend failure and keeps the modal open', async () => {
    mockOperatorApiCall.mockRejectedValueOnce(new Error('snapshot failed (400): invalid destination'));
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Snapshot' }));
    fireEvent.change(await screen.findByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i), {
      target: { value: 's3://bucket/snap.tar.gz' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Take Snapshot' }));

    expect(await screen.findByText('snapshot failed (400): invalid destination')).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i)).toBeInTheDocument();
  });

  it('closes the modal after a successful snapshot', async () => {
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'Snapshot' }));
    fireEvent.change(await screen.findByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i), {
      target: { value: 's3://bucket/snap.tar.gz' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Take Snapshot' }));

    await waitFor(() => {
      expect(mockToastSuccess).toHaveBeenCalledWith('Snapshot created');
    });
    await waitFor(() => {
      expect(screen.queryByPlaceholderText(/https:\/\/storage\.example\.com\/snapshots\//i)).not.toBeInTheDocument();
    });
  });

  it('auto-detects the SSH username when the SSH tab opens', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';
    mockOperatorApiCall.mockImplementation(async (action: string) => {
      if (action === 'ssh/user') {
        return jsonResponse({ success: true, username: 'sidecar' });
      }
      return jsonResponse({});
    });

    sandboxesRef.current = [makeSandbox({ sshPort: 2222 })];
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'SSH' }));

    await waitFor(() => {
      expect(mockOperatorApiCall).toHaveBeenCalledWith('ssh/user', undefined, { method: 'GET' });
    });
    expect(await screen.findByDisplayValue('sidecar')).toBeInTheDocument();
    expect(screen.getByText('Detected sandbox user: sidecar')).toBeInTheDocument();
  });

  it('preserves a manual username edit if detection finishes later', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';
    let resolveDetection: ((value: Response) => void) | undefined;
    const detectionPromise = new Promise<Response>((resolve) => {
      resolveDetection = resolve;
    });
    mockOperatorApiCall.mockImplementation(async (action: string) => {
      if (action === 'ssh/user') {
        return detectionPromise;
      }
      return jsonResponse({});
    });

    sandboxesRef.current = [makeSandbox({ sshPort: 2222 })];
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'SSH' }));
    const usernameInput = screen.getByLabelText('SSH username');
    fireEvent.change(usernameInput, { target: { value: 'custom-user' } });
    resolveDetection?.(jsonResponse({ success: true, username: 'sidecar' }));

    await waitFor(() => {
      expect(screen.getByDisplayValue('custom-user')).toBeInTheDocument();
    });
    expect(screen.getByText('Detected sandbox user: sidecar')).toBeInTheDocument();
  });

  it('does not add an SSH key when the backend returns an error', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';
    mockOperatorApiCall.mockImplementation(async (action: string) => {
      if (action === 'ssh/user') {
        return jsonResponse({ success: true, username: 'sidecar' });
      }
      if (action === 'ssh') {
        throw new Error('ssh failed (422): {"error":"SSH provision failed for user \'agent\' (exit 2): User agent does not exist"}');
      }
      return jsonResponse({});
    });

    sandboxesRef.current = [makeSandbox({ sshPort: 2222 })];
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'SSH' }));
    fireEvent.change(await screen.findByLabelText('SSH public key'), {
      target: { value: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest route@test' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add Key' }));

    expect(await screen.findByText("SSH provision failed for user 'agent' (exit 2): User agent does not exist")).toBeInTheDocument();
    expect(screen.queryByText('Active Keys')).not.toBeInTheDocument();
    expect(screen.queryByText('SSH key provisioned')).not.toBeInTheDocument();
  });

  it('hides SSH tab when sshPort is absent', () => {
    sandboxesRef.current = [makeSandbox()];
    renderSubject();
    expect(screen.queryByRole('button', { name: 'SSH' })).not.toBeInTheDocument();
  });

  it('shows SSH tab when sshPort is present', () => {
    sandboxesRef.current = [makeSandbox({ sshPort: 2222 })];
    renderSubject();
    expect(screen.getByRole('button', { name: 'SSH' })).toBeInTheDocument();
  });

  it('stores the backend-returned SSH username after a successful add', async () => {
    operatorAuthState.isAuthenticated = true;
    operatorAuthState.cachedToken = 'operator-token';
    mockOperatorApiCall.mockImplementation(async (action: string) => {
      if (action === 'ssh/user') {
        return jsonResponse({ success: true, username: 'sidecar' });
      }
      if (action === 'ssh') {
        return jsonResponse({
          success: true,
          username: 'sidecar',
          result: { result: { exitCode: 0, stdout: '', stderr: '' } },
        });
      }
      return jsonResponse({});
    });

    sandboxesRef.current = [makeSandbox({ sshPort: 2222 })];
    renderSubject();

    fireEvent.click(screen.getByRole('button', { name: 'SSH' }));
    fireEvent.change(await screen.findByLabelText('SSH username'), {
      target: { value: 'agent' },
    });
    fireEvent.change(screen.getByLabelText('SSH public key'), {
      target: { value: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest route@test' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add Key' }));

    expect(await screen.findByText('sidecar@')).toBeInTheDocument();
    expect(screen.getByDisplayValue('sidecar')).toBeInTheDocument();
  });
});
