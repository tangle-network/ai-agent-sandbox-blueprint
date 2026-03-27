import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Workflows from '../routes/workflows._index';

const {
  accountRef,
  pendingWorkflowsRef,
  workflowSummaryRef,
  instanceWorkflowSummaryRef,
  sandboxTokenRef,
  instanceTokenRef,
  submitStatusRef,
  fetchMock,
  submitJobMock,
  invalidateQueriesMock,
  useWorkflowSummariesMock,
  useWorkflowOperatorAccessMock,
  mockToastSuccess,
  mockToastError,
} = vi.hoisted(() => ({
  accountRef: {
    current: { address: '0x123400000000000000000000000000000000abcd' as `0x${string}` | undefined },
  },
  pendingWorkflowsRef: {
    current: [] as Array<Record<string, unknown>>,
  },
  workflowSummaryRef: {
    current: [] as Array<Record<string, unknown>>,
  },
  instanceWorkflowSummaryRef: {
    current: [] as Array<Record<string, unknown>>,
  },
  sandboxTokenRef: {
    current: 'sandbox-token' as string | null,
  },
  instanceTokenRef: {
    current: 'instance-token' as string | null,
  },
  submitStatusRef: {
    current: 'idle',
  },
  fetchMock: vi.fn(),
  submitJobMock: vi.fn(),
  invalidateQueriesMock: vi.fn().mockResolvedValue(undefined),
  useWorkflowSummariesMock: vi.fn(),
  useWorkflowOperatorAccessMock: vi.fn(),
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
  useNavigate: () => vi.fn(),
}));

vi.mock('@nanostores/react', () => ({
  useStore: (store: { get?: () => unknown }) => store.get?.() ?? [],
}));

vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({
    invalidateQueries: invalidateQueriesMock,
  }),
}));

vi.mock('wagmi', () => ({
  useAccount: () => accountRef.current,
}));

vi.mock('sonner', () => ({
  toast: {
    success: mockToastSuccess,
    error: mockToastError,
  },
}));

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  AnimatedPage: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  StaggerContainer: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  StaggerItem: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Card: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardContent: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Badge: ({ children, ...props }: { children: React.ReactNode }) => <span {...props}>{children}</span>,
}));

vi.mock('@tangle-network/blueprint-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tangle-network/blueprint-ui')>();
  return {
    ...actual,
    cn: (...classes: Array<string | false | null | undefined>) => classes.filter(Boolean).join(' '),
    getAddresses: () => ({
      sandboxBlueprint: '0x1111111111111111111111111111111111111111',
      instanceBlueprint: '0x2222222222222222222222222222222222222222',
      teeInstanceBlueprint: '0x3333333333333333333333333333333333333333',
    }),
    useSubmitJob: () => ({
      submitJob: submitJobMock,
      status: submitStatusRef.current,
    }),
    encodeJobArgs: vi.fn(() => '0xencoded'),
    getJobById: vi.fn(),
  };
});

vi.mock('viem', async (importOriginal) => {
  const actual = await importOriginal<typeof import('viem')>();
  return {
    ...actual,
  };
});

vi.mock('~/lib/contracts/chains', () => ({
  isContractDeployed: () => true,
}));

vi.mock('~/lib/config', () => ({
  OPERATOR_API_URL: 'https://sandbox.example',
  INSTANCE_OPERATOR_API_URL: 'https://instance.example',
}));

vi.mock('~/lib/stores/sandboxes', () => ({
  sandboxListStore: {
    get: () => [],
  },
}));

vi.mock('~/lib/stores/instances', () => ({
  instanceListStore: {
    get: () => [],
  },
}));

vi.mock('~/lib/stores/pendingWorkflows', () => ({
  pendingWorkflowStore: {
    get: () => pendingWorkflowsRef.current,
  },
  normalizeWorkflowOwnerAddress: (address: string | null | undefined) => (address || '').trim().toLowerCase(),
  updatePendingWorkflow: (key: string, patch: Record<string, unknown>) => {
    pendingWorkflowsRef.current = pendingWorkflowsRef.current.map((entry) => (
      entry.key === key ? { ...entry, ...patch } : entry
    ));
  },
  removePendingWorkflow: (key: string) => {
    pendingWorkflowsRef.current = pendingWorkflowsRef.current.filter((entry) => entry.key !== key);
  },
}));

vi.mock('~/lib/hooks/useWorkflowRuntimeStatus', () => ({
  useWorkflowSummaries: (...args: unknown[]) => useWorkflowSummariesMock(...args),
  useWorkflowOperatorAccess: (...args: unknown[]) => useWorkflowOperatorAccessMock(...args),
}));

function setDefaultWorkflowSummaryMocks() {
  useWorkflowSummariesMock.mockImplementation((operatorUrl: string) => ({
    data: operatorUrl === 'https://sandbox.example'
      ? workflowSummaryRef.current
      : instanceWorkflowSummaryRef.current,
    authRequired: !(operatorUrl === 'https://sandbox.example'
      ? sandboxTokenRef.current
      : instanceTokenRef.current),
    authError: null,
    error: null,
    isLoading: false,
    isAuthenticating: false,
    authenticate: vi.fn(async () => operatorUrl === 'https://sandbox.example'
      ? sandboxTokenRef.current
      : instanceTokenRef.current),
    refetch: vi.fn().mockResolvedValue(undefined),
  }));

  useWorkflowOperatorAccessMock.mockImplementation((operatorUrl: string) => ({
    operatorUrl,
    authCacheKey: `${operatorUrl}::wallet`,
    getCachedToken: () => (operatorUrl === 'https://sandbox.example'
      ? sandboxTokenRef.current
      : instanceTokenRef.current),
    getToken: vi.fn(async () => (operatorUrl === 'https://sandbox.example'
      ? sandboxTokenRef.current
      : instanceTokenRef.current)),
    authenticate: vi.fn(async () => {
      if (operatorUrl === 'https://sandbox.example') {
        sandboxTokenRef.current = sandboxTokenRef.current ?? 'sandbox-token';
        return sandboxTokenRef.current;
      }
      instanceTokenRef.current = instanceTokenRef.current ?? 'instance-token';
      return instanceTokenRef.current;
    }),
    authRequired: !(operatorUrl === 'https://sandbox.example'
      ? sandboxTokenRef.current
      : instanceTokenRef.current),
    isAuthenticated: true,
    isAuthenticating: false,
    authError: null,
  }));
}

describe('Workflows list', () => {
  beforeEach(() => {
    vi.useRealTimers();
    pendingWorkflowsRef.current = [];
    workflowSummaryRef.current = [];
    instanceWorkflowSummaryRef.current = [];
    sandboxTokenRef.current = 'sandbox-token';
    instanceTokenRef.current = 'instance-token';
    submitStatusRef.current = 'idle';
    fetchMock.mockReset();
    submitJobMock.mockReset();
    invalidateQueriesMock.mockClear();
    useWorkflowSummariesMock.mockReset();
    useWorkflowOperatorAccessMock.mockReset();

    fetchMock.mockResolvedValue(new Response(null, { status: 404 }));
    vi.stubGlobal('fetch', fetchMock);

    setDefaultWorkflowSummaryMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('shows missing-target workflows as not runnable and disables trigger', () => {
    workflowSummaryRef.current = [{
      scope: 'sandbox',
      workflowId: 42,
      name: 'Nightly Summary',
      triggerType: 'cron',
      triggerConfig: '*/30 * * * * *',
      targetKind: 0,
      targetSandboxId: 'sandbox-1',
      targetServiceId: 7,
      active: true,
      targetStatus: 'missing',
      runnable: false,
      running: false,
      lastRunAt: 1710000000,
      nextRunAt: null,
      latestExecution: null,
    }];

    render(<Workflows />);

    expect(screen.getByText('Not Runnable')).toBeInTheDocument();
    expect(screen.getByText('Target missing: sandbox-1')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Trigger' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  });

  it('renders a timed-out pending workflow as informational instead of failed', () => {
    pendingWorkflowsRef.current = [{
      key: '0x123400000000000000000000000000000000abcd::sandbox::101',
      ownerAddress: '0x123400000000000000000000000000000000abcd',
      workflowId: 101,
      scope: 'sandbox',
      blueprintId: 'ai-agent-sandbox-blueprint',
      operatorUrl: 'https://sandbox.example',
      name: 'Slow Workflow',
      triggerType: 'cron',
      triggerConfig: '',
      targetKind: 0,
      targetSandboxId: 'sandbox-1',
      targetServiceId: 7,
      targetLabel: 'Target Sandbox',
      kindLabel: 'Sandbox',
      txHash: '0xabc123',
      createdAt: Date.now(),
      submittedAt: Date.now() - 121_000,
      status: 'timed-out',
      statusMessage: 'Creation is still processing. Use Check Status to try again.',
    }];

    render(<Workflows />);

    expect(screen.getByText('Still Processing')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Check Status' })).toBeInTheDocument();
    expect(screen.queryByText(/workflow creation failed/i)).not.toBeInTheDocument();
  });

  it('renders a link to the create page', () => {
    render(<Workflows />);

    const link = screen.getByRole('link', { name: /New Workflow/i });
    expect(link).toHaveAttribute('href', '/workflows/create');
  });
});
