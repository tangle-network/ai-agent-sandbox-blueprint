import { fireEvent, render, screen, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ConsoleShell } from './ConsoleShell';

const hoisted = vi.hoisted(() => ({
  walletProps: [] as Array<Record<string, unknown>>,
  chainProps: [] as Array<Record<string, unknown>>,
  txProps: [] as Array<Record<string, unknown>>,
}));

vi.mock('@tangle-network/blueprint-ui', () => ({
  cn: (...values: Array<string | false | null | undefined>) => values.filter(Boolean).join(' '),
}));

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  ThemeToggle: () => <button type="button">Theme</button>,
}));

vi.mock('~/components/shared/TangleBrand', () => ({
  TangleBrandLogo: ({ compact = false }: { compact?: boolean }) => (
    <img src={compact ? '/tangle-mark.svg' : '/tangle-logo-light.svg'} alt="" />
  ),
}));

vi.mock('~/components/layout/WalletButton', () => ({
  WalletButton: (props: Record<string, unknown>) => {
    hoisted.walletProps.push(props);
    return (
      <button type="button" data-compact={props.compact ? 'true' : 'false'}>
        {props.compact ? 'Wallet compact' : 'Wallet expanded'}
      </button>
    );
  },
}));

vi.mock('~/components/console/ConsoleChainSwitcher', () => ({
  ConsoleChainSwitcher: (props: Record<string, unknown>) => {
    hoisted.chainProps.push(props);
    return (
      <button type="button" data-compact={props.compact ? 'true' : 'false'}>
        {props.compact ? 'Network compact' : 'Network expanded'}
      </button>
    );
  },
}));

vi.mock('~/components/layout/TxDropdown', () => ({
  TxDropdown: (props: Record<string, unknown>) => {
    hoisted.txProps.push(props);
    return <button type="button">Transactions</button>;
  },
}));

function renderShell(path = '/') {
  window.history.pushState({}, '', path);
  render(
    <MemoryRouter initialEntries={[path]}>
      <ConsoleShell>
        <div>Console body</div>
      </ConsoleShell>
    </MemoryRouter>,
  );
}

describe('ConsoleShell', () => {
  beforeEach(() => {
    window.localStorage.clear();
    hoisted.walletProps = [];
    hoisted.chainProps = [];
    hoisted.txProps = [];
  });

  it('renders the expanded command dock as wallet, network, transactions, and theme controls', () => {
    renderShell('/create');

    const sidebar = screen.getByRole('navigation', { name: 'Tangle sandbox navigation' }).closest('aside');

    expect(sidebar).not.toBeNull();
    expect(sidebar).toHaveClass('w-[268px]');
    expect(within(sidebar!).getByRole('button', { name: /collapse sidebar/i })).toBeInTheDocument();
    expect(within(sidebar!).getByRole('button', { name: 'Wallet expanded' })).toBeInTheDocument();
    expect(within(sidebar!).getByRole('button', { name: 'Network expanded' })).toBeInTheDocument();
    expect(within(sidebar!).getByRole('button', { name: 'Transactions' })).toBeInTheDocument();
    expect(within(sidebar!).getByRole('button', { name: 'Theme' })).toBeInTheDocument();

    expect(hoisted.walletProps).toContainEqual(expect.objectContaining({ align: 'start', side: 'up' }));
    expect(hoisted.chainProps).toContainEqual(expect.objectContaining({ align: 'start', placement: 'up' }));
    expect(hoisted.txProps).toContainEqual(expect.objectContaining({ align: 'start', side: 'up', compact: true }));
  });

  it('persists the collapsed desktop sidebar and switches the dock to compact account controls', async () => {
    renderShell('/sandboxes');

    fireEvent.click(screen.getByRole('button', { name: /collapse sidebar/i }));

    const sidebar = screen.getByRole('navigation', { name: 'Tangle sandbox navigation' }).closest('aside');

    expect(window.localStorage.getItem('sandbox:console-sidebar-collapsed')).toBe('true');
    expect(sidebar).not.toBeNull();
    expect(sidebar).toHaveClass('w-16');
    expect(within(sidebar!).getByRole('button', { name: /expand sidebar/i })).toBeInTheDocument();
    expect(within(sidebar!).getByRole('link', { name: 'Sandboxes' })).toHaveAttribute('title', 'Sandboxes');
    expect(within(sidebar!).getByRole('button', { name: 'Wallet compact' })).toBeInTheDocument();
    expect(within(sidebar!).getByRole('button', { name: 'Network compact' })).toBeInTheDocument();
    expect(within(sidebar!).queryByRole('button', { name: 'Transactions' })).not.toBeInTheDocument();

    expect(hoisted.walletProps).toContainEqual(expect.objectContaining({ align: 'start', side: 'up', compact: true }));
    expect(hoisted.chainProps).toContainEqual(expect.objectContaining({ align: 'start', placement: 'up', compact: true }));
  });

  it('hydrates from the persisted collapsed preference', () => {
    window.localStorage.setItem('sandbox:console-sidebar-collapsed', 'true');

    renderShell('/');

    const sidebar = screen.getByRole('navigation', { name: 'Tangle sandbox navigation' }).closest('aside');

    expect(sidebar).not.toBeNull();
    expect(sidebar).toHaveClass('w-16');
    expect(within(sidebar!).getByRole('button', { name: /expand sidebar/i })).toBeInTheDocument();
  });
});
