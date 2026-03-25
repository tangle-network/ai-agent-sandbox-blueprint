import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Workflows from '../routes/workflows._index';

const {
  accountRef,
  workflowSummaryRef,
  useWorkflowSummariesMock,
} = vi.hoisted(() => ({
  accountRef: {
    current: { address: '0x123400000000000000000000000000000000abcd' as `0x${string}` | undefined },
  },
  workflowSummaryRef: {
    current: [] as Array<Record<string, unknown>>,
  },
  useWorkflowSummariesMock: vi.fn(),
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
  useSearchParams: () => [new URLSearchParams()],
}));

vi.mock('@nanostores/react', () => ({
  useStore: () => [],
}));

vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({
    invalidateQueries: vi.fn().mockResolvedValue(undefined),
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
  Select: ({ options, value, onChange }: { options: Array<{ value: string; label: string }>; value: string; onChange: (value: string) => void }) => (
    <select value={value} onChange={(event) => onChange(event.target.value)}>
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
    publicClient: {},
    tangleJobsAbi: [],
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

vi.mock('~/lib/config', () => ({
  OPERATOR_API_URL: 'https://sandbox.example',
  INSTANCE_OPERATOR_API_URL: 'https://instance.example',
}));

vi.mock('~/lib/hooks/useWorkflowRuntimeStatus', () => ({
  useWorkflowSummaries: (...args: unknown[]) => useWorkflowSummariesMock(...args),
}));

describe('Workflows list', () => {
  beforeEach(() => {
    useWorkflowSummariesMock.mockImplementation((operatorUrl: string) => ({
      data: operatorUrl === 'https://sandbox.example' ? workflowSummaryRef.current : [],
      authRequired: false,
      authError: null,
      error: null,
      isLoading: false,
      isAuthenticating: false,
      authenticate: vi.fn(),
      refetch: vi.fn(),
    }));
    workflowSummaryRef.current = [];
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
});
