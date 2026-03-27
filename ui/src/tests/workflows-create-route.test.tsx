import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import WorkflowCreate from '../routes/workflows.create';

const {
  accountRef,
  sandboxesRef,
  instancesRef,
  pendingWorkflowsRef,
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
  navigateMock,
  mockToastSuccess,
  mockToastError,
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
  navigateMock: vi.fn(),
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
  useNavigate: () => navigateMock,
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

async function fillAndSubmitForm(name = 'Nightly Summary') {
  const nameInput = screen.getByPlaceholderText('daily-backup');
  fireEvent.change(nameInput, {
    target: { value: name },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: /Create Workflow/i }));
  });
}

describe('Workflow create page', () => {
  beforeEach(() => {
    vi.useRealTimers();
    sandboxesRef.current = [];
    instancesRef.current = [];
    pendingWorkflowsRef.current = [];
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
    navigateMock.mockReset();

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
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('shows success state with View Workflow button after creation', async () => {
    makeTargetSandbox();

    render(<WorkflowCreate />);
    await fillAndSubmitForm();

    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByRole('button', { name: /View Workflow/i })).toBeInTheDocument();
    expect(pendingWorkflowsRef.current).toHaveLength(1);
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

    render(<WorkflowCreate />);
    await fillAndSubmitForm('Broken Receipt');

    expect(await screen.findByText(/workflow call ID could not be found/i)).toBeInTheDocument();
    expect(pendingWorkflowsRef.current).toHaveLength(0);
  });

  it('still hard-fails when the workflow creation transaction reverts', async () => {
    makeTargetSandbox();
    waitForTransactionReceiptMock.mockResolvedValue({
      status: 'reverted',
      logs: [],
    });

    render(<WorkflowCreate />);
    await fillAndSubmitForm('Reverted Workflow');

    expect(await screen.findByText(/transaction reverted/i)).toBeInTheDocument();
    expect(pendingWorkflowsRef.current).toHaveLength(0);
  });

  it('has a Go Back link to workflows list', () => {
    render(<WorkflowCreate />);

    const backLink = screen.getByRole('link', { name: /Go Back/i });
    expect(backLink).toHaveAttribute('href', '/workflows');
  });

  it('shows no runnable targets warning when none are available', () => {
    render(<WorkflowCreate />);

    expect(screen.getByText('No runnable targets available')).toBeInTheDocument();
  });

  it('shows no-agent warning when selected target lacks agentIdentifier', () => {
    sandboxesRef.current = [{
      localId: 'sandbox-local-1',
      sandboxId: 'sandbox-1',
      name: 'Agent-less Sandbox',
      image: 'ghcr.io/example/compute:latest',
      serviceId: '7',
      status: 'running',
    }];

    render(<WorkflowCreate />);

    expect(screen.getByText('No agent configured on this target')).toBeInTheDocument();
  });

  it('does not show no-agent warning when target has agentIdentifier', () => {
    sandboxesRef.current = [{
      localId: 'sandbox-local-1',
      sandboxId: 'sandbox-1',
      name: 'Agent Sandbox',
      image: 'ghcr.io/example/agent:latest',
      serviceId: '7',
      status: 'running',
      agentIdentifier: 'default',
    }];

    render(<WorkflowCreate />);

    expect(screen.queryByText('No agent configured on this target')).not.toBeInTheDocument();
  });

  it('shows (no agent) suffix in dropdown for agent-less targets', () => {
    sandboxesRef.current = [{
      localId: 'sandbox-local-1',
      sandboxId: 'sandbox-1',
      name: 'Plain Box',
      image: 'ghcr.io/example/compute:latest',
      serviceId: '7',
      status: 'running',
    }];

    render(<WorkflowCreate />);

    const option = screen.getByRole('option', { name: /Plain Box/ });
    expect(option.textContent).toContain('(no agent)');
  });

  it('does not show (no agent) suffix when target has agent', () => {
    sandboxesRef.current = [{
      localId: 'sandbox-local-1',
      sandboxId: 'sandbox-1',
      name: 'Agent Box',
      image: 'ghcr.io/example/agent:latest',
      serviceId: '7',
      status: 'running',
      agentIdentifier: 'default',
    }];

    render(<WorkflowCreate />);

    const option = screen.getByRole('option', { name: /Agent Box/ });
    expect(option.textContent).not.toContain('(no agent)');
  });
});
