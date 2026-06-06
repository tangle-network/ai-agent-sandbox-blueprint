import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import OperatorCapacity from './operators._index';

const hoisted = vi.hoisted(() => ({
  sandboxes: [
    {
      operator: '0x1111111111111111111111111111111111111111',
      status: 'running',
      teeEnabled: false,
      createdAt: 1000,
      lastActivityAt: 2000,
    },
  ],
  instances: [
    {
      operator: '0x2222222222222222222222222222222222222222',
      status: 'creating',
      teeEnabled: true,
      createdAt: 3000,
    },
  ],
  capacity: 9,
}));

vi.mock('react-router', () => ({
  Link: ({ children, to, ...props }: { children: ReactNode; to: string }) => (
    <a href={to} {...props}>{children}</a>
  ),
}));

vi.mock('@nanostores/react', () => ({
  useStore: (store: { get?: () => unknown }) => store.get?.() ?? [],
}));

vi.mock('@tangle-network/blueprint-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tangle-network/blueprint-ui')>();
  return {
    ...actual,
    cn: (...classes: Array<string | false | null | undefined>) => classes.filter(Boolean).join(' '),
  };
});

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
}));

vi.mock('~/lib/stores/sandboxes', () => ({
  sandboxListStore: {
    get: () => hoisted.sandboxes,
  },
}));

vi.mock('~/lib/stores/instances', () => ({
  instanceListStore: {
    get: () => hoisted.instances,
  },
}));

vi.mock('~/lib/hooks/useSandboxReads', () => ({
  useAvailableCapacity: () => ({ data: hoisted.capacity }),
}));

vi.mock('~/lib/config', () => ({
  SANDBOX_ONCHAIN_BLUEPRINT_ID: '10',
  INSTANCE_ONCHAIN_BLUEPRINT_ID: '11',
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID: '12',
}));

vi.mock('~/lib/hooks/useReliableOperators', () => ({
  useReliableOperators: (blueprintId: string) => {
    const operatorByBlueprint: Record<string, string[]> = {
      '10': ['0x1111111111111111111111111111111111111111'],
      '11': ['0x2222222222222222222222222222222222222222'],
      '12': [],
    };
    const addresses = operatorByBlueprint[blueprintId] ?? [];
    return {
      operators: addresses.map((address) => ({
        address,
        rpcAddress: `https://operator-${blueprintId}.example`,
      })),
      isLoading: false,
      listError: null,
      source: 'events',
      operatorCount: BigInt(addresses.length),
    };
  },
}));

describe('OperatorCapacity', () => {
  beforeEach(() => {
    hoisted.capacity = 9;
  });

  it('removes internal sourcing labels from primary metrics', () => {
    render(<OperatorCapacity />);

    expect(screen.getByText('Available slots')).toBeInTheDocument();
    expect(screen.queryByText(/BSM capacity/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/on-chain counts/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/service-verified/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/event index/i)).not.toBeInTheDocument();
  });

  it('filters the operator table by blueprint', () => {
    render(<OperatorCapacity />);

    fireEvent.click(screen.getByRole('button', { name: /Instance 1/i }));

    const table = screen.getByRole('table');
    expect(within(table).getByText('Instance')).toBeInTheDocument();
    expect(within(table).queryByText('Sandbox')).not.toBeInTheDocument();
  });
});
