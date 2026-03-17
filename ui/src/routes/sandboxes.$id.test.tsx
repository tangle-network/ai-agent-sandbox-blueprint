import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SandboxDetail from './sandboxes.$id';

const {
  sandboxesRef,
  mockNavigate,
  mockOperatorApiCall,
  mockUpdateSandboxStatus,
  mockToastSuccess,
  mockToastError,
  mockSubmitJob,
} = vi.hoisted(() => ({
  sandboxesRef: { current: [] as Array<Record<string, unknown>> },
  mockNavigate: vi.fn(),
  mockOperatorApiCall: vi.fn(),
  mockUpdateSandboxStatus: vi.fn(),
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
  mockSubmitJob: vi.fn(),
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
  useOperatorAuth: () => ({ getToken: vi.fn().mockResolvedValue('operator-token') }),
}));

vi.mock('~/lib/hooks/useOperatorApiCall', () => ({
  useOperatorApiCall: () => mockOperatorApiCall,
}));

vi.mock('~/lib/hooks/useWagmiSidecarAuth', () => ({
  useWagmiSidecarAuth: () => ({
    token: null,
    isAuthenticated: false,
    authenticate: vi.fn(),
    isAuthenticating: false,
  }),
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
  ResourceTabs: () => <div>Tabs</div>,
}));

vi.mock('~/components/shared/ProvisionProgress', () => ({
  ProvisionProgress: () => <div>Provision Progress</div>,
}));

vi.mock('~/components/shared/JobPriceBadge', () => ({
  JobPriceBadge: () => <span>Price</span>,
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

describe('SandboxDetail snapshot flow', () => {
  beforeEach(() => {
    sandboxesRef.current = [makeSandbox()];
    mockNavigate.mockReset();
    mockOperatorApiCall.mockReset();
    mockUpdateSandboxStatus.mockReset();
    mockToastSuccess.mockReset();
    mockToastError.mockReset();
    mockSubmitJob.mockReset();
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
});
