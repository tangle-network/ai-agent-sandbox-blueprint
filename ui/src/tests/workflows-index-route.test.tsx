import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Workflows from '../routes/workflows._index';

const {
  accountRef,
  sandboxesRef,
  instancesRef,
  pendingWorkflowsRef,
  workflowSummaryRef,
  instanceWorkflowSummaryRef,
  sandboxTokenRef,
  instanceTokenRef,
  submitStatusRef,
  fetchMock,
  submitJobMock,
  waitForTransactionReceiptMock,
  invalidateQueriesMock,
  decodeEventLogMock,
  getJobByIdMock,
  encodeJobArgsMock,
  useWorkflowSummariesMock,
  useWorkflowOperatorAccessMock,
} = vi.hoisted(() => ({
  accountRef: {
    current: { address: '0x123400000000000000000000000000000000abcd' as `0x${string}` | undefined },
  },
  sandboxesRef: {
    current: [] as Array<Record<string, unknown>>,
  },
  instancesRef: {
    current: [] as Array<Record<string, unknown>>,
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
  waitForTransactionReceiptMock: vi.fn(),
  invalidateQueriesMock: vi.fn().mockResolvedValue(undefined),
  decodeEventLogMock: vi.fn(),
  getJobByIdMock: vi.fn(),
  encodeJobArgsMock: vi.fn(() => '0xencoded'),
  useWorkflowSummariesMock: vi.fn(),
  useWorkflowOperatorAccessMock: vi.fn(),
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
  useSearchParams: () => [new URLSearchParams()],
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

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  AnimatedPage: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  StaggerContainer: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  StaggerItem: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Card: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardContent: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardHeader: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardTitle: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardDescription: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Badge: ({ children, ...props }: { children: React.ReactNode }) => <span {...props}>{children}</span>,
  Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
  Select: ({
    options,
    value,
    onValueChange,
  }: {
    options: Array<{ value: string; label: string }>;
    value: string;
    onValueChange: (value: string) => void;
  }) => (
    <select value={value} onChange={(event) => onValueChange(event.target.value)}>
      {options.map((option) => (
        <option key={option.value} value={option.value}>{option.label}</option>
      ))}
    </select>
  ),
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
    publicClient: {
      waitForTransactionReceipt: waitForTransactionReceiptMock,
    },
    tangleJobsAbi: [],
    useSubmitJob: () => ({
      submitJob: submitJobMock,
      status: submitStatusRef.current,
    }),
    encodeJobArgs: encodeJobArgsMock,
    getJobById: getJobByIdMock,
  };
});

vi.mock('viem', async (importOriginal) => {
  const actual = await importOriginal<typeof import('viem')>();
  return {
    ...actual,
    decodeEventLog: decodeEventLogMock,
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
    get: () => sandboxesRef.current,
  },
}));

vi.mock('~/lib/stores/instances', () => ({
  instanceListStore: {
    get: () => instancesRef.current,
  },
}));

vi.mock('~/lib/stores/pendingWorkflows', () => ({
  pendingWorkflowStore: {
    get: () => pendingWorkflowsRef.current,
  },
  normalizeWorkflowOwnerAddress: (address: string | null | undefined) => (address || '').trim().toLowerCase(),
  buildPendingWorkflowKey: (ownerAddress: string, scope: string, workflowId: number) =>
    `${(ownerAddress || '').trim().toLowerCase()}::${scope}::${workflowId}`,
  addPendingWorkflow: (entry: Record<string, unknown>) => {
    pendingWorkflowsRef.current = [
      entry,
      ...pendingWorkflowsRef.current.filter((record) => record.key !== entry.key),
    ];
  },
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

function makeTargetSandbox() {
  sandboxesRef.current = [{
    localId: 'sandbox-local-1',
    sandboxId: 'sandbox-1',
    name: 'Target Sandbox',
    image: 'ghcr.io/example/agent:latest',
    serviceId: '7',
    status: 'running',
  }];
}

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

async function createWorkflow(name = 'Nightly Summary') {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'New Workflow' }));
    await Promise.resolve();
  });
  const nameInput = screen.getByPlaceholderText('daily-backup');
  fireEvent.change(nameInput, {
    target: { value: name },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Create Workflow' }));
}

describe('Workflows list', () => {
  beforeEach(() => {
    vi.useRealTimers();
    sandboxesRef.current = [];
    instancesRef.current = [];
    pendingWorkflowsRef.current = [];
    workflowSummaryRef.current = [];
    instanceWorkflowSummaryRef.current = [];
    sandboxTokenRef.current = 'sandbox-token';
    instanceTokenRef.current = 'instance-token';
    submitStatusRef.current = 'idle';
    fetchMock.mockReset();
    submitJobMock.mockReset();
    waitForTransactionReceiptMock.mockReset();
    invalidateQueriesMock.mockClear();
    decodeEventLogMock.mockReset();
    getJobByIdMock.mockReset();
    encodeJobArgsMock.mockClear();
    useWorkflowSummariesMock.mockReset();
    useWorkflowOperatorAccessMock.mockReset();

    getJobByIdMock.mockReturnValue({ id: 2, name: 'workflow_create' });
    submitJobMock.mockResolvedValue('0xabc123');
    waitForTransactionReceiptMock.mockResolvedValue({
      status: 'success',
      logs: [{ topics: ['job-submitted'], data: '0x01' }],
    });
    decodeEventLogMock.mockImplementation(({ topics }: { topics: string[] }) => {
      if (topics[0] === 'job-submitted') {
        return {
          eventName: 'JobSubmitted',
          args: { callId: 101n },
        };
      }
      throw new Error('no match');
    });
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

  it('keeps a newly submitted workflow from hard-failing before the operator exposes it', async () => {
    vi.useFakeTimers();
    makeTargetSandbox();
    let visibilityChecks = 0;
    fetchMock.mockImplementation(async () => {
      visibilityChecks += 1;
      if (visibilityChecks < 4) {
        return new Response(null, { status: 404 });
      }

      workflowSummaryRef.current = [{
        scope: 'sandbox',
        workflowId: 101,
        name: 'Nightly Summary',
        triggerType: 'cron',
        triggerConfig: '',
        targetKind: 0,
        targetSandboxId: 'sandbox-1',
        targetServiceId: 7,
        active: true,
        targetStatus: 'available',
        runnable: true,
        running: false,
        lastRunAt: null,
        nextRunAt: 1710000300,
        latestExecution: null,
      }];

      return new Response(JSON.stringify({ workflowId: 101 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    });

    render(<Workflows />);
    await createWorkflow();
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.queryByText(/workflow creation failed/i)).not.toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(6_000);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.queryByText('Processing')).not.toBeInTheDocument();
    expect(screen.getByText('Active')).toBeInTheDocument();
    expect(screen.getAllByText('Nightly Summary').length).toBeGreaterThan(0);
  });

  it('shows a submitted state instead of a hard failure when operator auth is missing', async () => {
    makeTargetSandbox();
    sandboxTokenRef.current = null;
    setDefaultWorkflowSummaryMocks();

    render(<Workflows />);
    await createWorkflow('Needs Auth');

    expect(await screen.findByText('Submitted')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Connect Operator' })).toBeInTheDocument();
    expect(screen.getByText(/Connect to the sandbox operator to verify/i)).toBeInTheDocument();
    expect(screen.queryByText(/workflow creation failed/i)).not.toBeInTheDocument();
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

  it('still hard-fails when the confirmed receipt does not contain JobSubmitted', async () => {
    makeTargetSandbox();
    waitForTransactionReceiptMock.mockResolvedValue({
      status: 'success',
      logs: [{ topics: ['unrelated-event'], data: '0x02' }],
    });
    decodeEventLogMock.mockImplementation(() => {
      throw new Error('no match');
    });

    render(<Workflows />);
    await createWorkflow('Broken Receipt');

    expect(await screen.findByText(/workflow call ID could not be found/i)).toBeInTheDocument();
    expect(pendingWorkflowsRef.current).toHaveLength(0);
  });

  it('still hard-fails when the workflow creation transaction reverts', async () => {
    makeTargetSandbox();
    waitForTransactionReceiptMock.mockResolvedValue({
      status: 'reverted',
      logs: [],
    });

    render(<Workflows />);
    await createWorkflow('Reverted Workflow');

    expect(await screen.findByText(/transaction reverted/i)).toBeInTheDocument();
    expect(pendingWorkflowsRef.current).toHaveLength(0);
  });
});
