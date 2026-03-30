import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import WorkflowDetail from './workflows.$scope.$workflowId';

const {
  accountRef,
  workflowDetailQueryRef,
  mockToastSuccess,
  mockToastError,
} = vi.hoisted(() => ({
  accountRef: {
    current: { address: undefined as `0x${string}` | undefined },
  },
  workflowDetailQueryRef: {
    current: {
      data: null as Record<string, unknown> | null,
      authRequired: false,
      isLoading: false,
      error: null as Error | null,
      isAuthenticating: false,
      authenticate: vi.fn(),
      refetch: vi.fn(),
    },
  },
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
  useParams: () => ({ scope: 'sandbox', workflowId: '1' }),
}));

vi.mock('@nanostores/react', () => ({
  useStore: () => [],
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

vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({
    invalidateQueries: vi.fn().mockResolvedValue(undefined),
  }),
}));

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  AnimatedPage: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Card: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardContent: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardHeader: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardTitle: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardDescription: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Badge: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
}));

vi.mock('@tangle-network/blueprint-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tangle-network/blueprint-ui')>();
  return {
    ...actual,
    getAddresses: () => ({
      sandboxBlueprint: '0x1111111111111111111111111111111111111111',
      instanceBlueprint: '0x2222222222222222222222222222222222222222',
      teeInstanceBlueprint: '0x3333333333333333333333333333333333333333',
      jobs: '0x4444444444444444444444444444444444444444',
      services: '0x5555555555555555555555555555555555555555',
    }),
    useSubmitJob: () => ({
      submitJob: vi.fn(),
      status: 'idle',
    }),
    encodeJobArgs: vi.fn(),
    getJobById: vi.fn(),
  };
});

vi.mock('~/lib/contracts/chains', () => ({
  isContractDeployed: () => true,
}));

vi.mock('~/lib/types/sandbox', () => ({
  JOB_IDS: { WORKFLOW_TRIGGER: 3, WORKFLOW_CANCEL: 4 },
  PRICING_TIERS: {},
}));

vi.mock('~/lib/stores/sandboxes', () => ({
  sandboxListStore: { get: () => [] },
}));

vi.mock('~/lib/stores/instances', () => ({
  instanceListStore: { get: () => [] },
}));

vi.mock('~/lib/hooks/useSandboxReads', () => ({
  useWorkflowForAddress: () => ({ data: null, isLoading: false, error: null }),
}));

vi.mock('~/lib/hooks/useWorkflowRuntimeStatus', () => ({
  useWorkflowDetail: () => workflowDetailQueryRef.current,
}));

vi.mock('~/lib/workflows', () => ({
  getWorkflowBlueprintIdForScope: (scope: string) => `ai-agent-${scope}-blueprint`,
  resolveWorkflowTargetLabelFromValues: () => ({ label: 'Test Target', kindLabel: 'Sandbox' }),
}));

describe('WorkflowDetail access control', () => {
  beforeEach(() => {
    accountRef.current = { address: undefined };
    workflowDetailQueryRef.current = {
      data: null,
      authRequired: false,
      isLoading: false,
      error: null,
      isAuthenticating: false,
      authenticate: vi.fn(),
      refetch: vi.fn(),
    };
  });

  it('asks for the owner wallet before showing workflow details', () => {
    render(<WorkflowDetail />);

    expect(
      screen.getByText('Connect the wallet that owns this workflow to view its details.'),
    ).toBeInTheDocument();
  });

  it('asks for operator auth before loading workflow details', () => {
    accountRef.current = {
      address: '0x123400000000000000000000000000000000abcd',
    };
    workflowDetailQueryRef.current = {
      ...workflowDetailQueryRef.current,
      authRequired: true,
    };

    render(<WorkflowDetail />);

    expect(
      screen.getByText('Authenticate with the operator to load this workflow.'),
    ).toBeInTheDocument();
  });

  it('renders workflow detail data when the operator returns the owned workflow', () => {
    accountRef.current = {
      address: '0x123400000000000000000000000000000000abcd',
    };
    workflowDetailQueryRef.current = {
      ...workflowDetailQueryRef.current,
      data: {
        scope: 'sandbox',
        workflowId: 1,
        name: 'Nightly Summary',
        triggerType: 'cron',
        triggerConfig: '*/30 * * * * *',
        targetKind: 0,
        targetSandboxId: 'sandbox-1',
        targetServiceId: 7,
        active: true,
        targetStatus: 'available',
        runnable: true,
        running: false,
        lastRunAt: 1710000000,
        nextRunAt: 1710000030,
        latestExecution: null,
        workflowJson: '{"prompt":"hello"}',
        sandboxConfigJson: '{}',
      },
    };

    render(<WorkflowDetail />);

    expect(
      screen.getByText('Nightly Summary'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('Target Service'),
    ).toBeInTheDocument();
    expect(
      screen.queryByText('Workflow not found for the connected wallet.'),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText('Authenticate with the operator to load this workflow.'),
    ).not.toBeInTheDocument();
  });

  it('renders orphaned workflows as not runnable', () => {
    accountRef.current = {
      address: '0x123400000000000000000000000000000000abcd',
    };
    workflowDetailQueryRef.current = {
      ...workflowDetailQueryRef.current,
      data: {
        scope: 'sandbox',
        workflowId: 1,
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
        workflowJson: '{"prompt":"hello"}',
        sandboxConfigJson: '{}',
      },
    };

    render(<WorkflowDetail />);

    expect(screen.getAllByText('Not Runnable').length).toBeGreaterThan(0);
    expect(
      screen.getByText('This workflow cannot run because its target sandbox or instance is no longer available.'),
    ).toBeInTheDocument();
  });

  it('shows a not found message when the operator cannot load the workflow', () => {
    accountRef.current = {
      address: '0x123400000000000000000000000000000000abcd',
    };

    render(<WorkflowDetail />);

    expect(
      screen.getByText('Workflow not found'),
    ).toBeInTheDocument();
  });
});
