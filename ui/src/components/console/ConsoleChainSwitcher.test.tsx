import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ConsoleChainSwitcher } from './ConsoleChainSwitcher';

const hoisted = vi.hoisted(() => ({
  selectedChainId: 84532,
  setChainId: vi.fn(),
}));

vi.mock('@nanostores/react', () => ({
  useStore: (store: { get?: () => unknown }) => store.get?.() ?? hoisted.selectedChainId,
}));

vi.mock('@tangle-network/blueprint-ui', () => ({
  cn: (...values: Array<string | false | null | undefined>) => values.filter(Boolean).join(' '),
  selectedChainIdStore: {
    get: () => hoisted.selectedChainId,
    set: (value: number) => {
      hoisted.selectedChainId = value;
      hoisted.setChainId(value);
    },
    subscribe: vi.fn(() => () => undefined),
  },
  getNetworks: () => ({
    84532: {
      label: 'Base Sepolia',
      shortLabel: 'Base',
      chain: { id: 84532, name: 'Base Sepolia' },
    },
    1: {
      label: 'Tangle Mainnet',
      shortLabel: 'Mainnet',
      chain: { id: 1, name: 'Tangle' },
    },
  }),
}));

describe('ConsoleChainSwitcher', () => {
  beforeEach(() => {
    hoisted.selectedChainId = 84532;
    hoisted.setChainId.mockClear();
  });

  it('keeps compact triggers icon-only while exposing the selected network in the tooltip', () => {
    render(<ConsoleChainSwitcher compact align="start" placement="up" />);

    const trigger = screen.getByRole('button', { name: 'Network' });

    expect(trigger).toHaveClass('w-10');
    expect(trigger).toHaveAttribute('title', 'Base Sepolia');
    expect(screen.queryByText('Base Sepolia')).not.toBeInTheDocument();
  });

  it('opens a custom upward menu with clamped dimensions from sidebar placement', async () => {
    render(<ConsoleChainSwitcher align="start" placement="up" />);

    fireEvent.click(screen.getByRole('button', { name: 'Network' }));

    const menu = screen.getByRole('menu');

    expect(menu).toHaveClass('left-0');
    expect(menu).toHaveClass('bottom-full');
    expect(menu).toHaveClass('mb-2');
    expect(menu).toHaveClass('w-[min(18rem,calc(100vw-1rem))]');
    expect(menu).toHaveClass('max-h-[min(24rem,calc(100vh-1rem))]');
    expect(screen.getByRole('menuitemradio', { name: /Base Sepolia.*84532/i })).toHaveAttribute('aria-checked', 'true');
  });
});
