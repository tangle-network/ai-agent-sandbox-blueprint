import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { WalletButton } from './WalletButton';

const hoisted = vi.hoisted(() => ({
  account: {
    address: '0x1111111111111111111111111111111111111111' as `0x${string}` | undefined,
    chainId: 84532,
    isConnected: true,
    status: 'connected',
  },
  dropdownOpen: false,
  toggle: vi.fn(),
  close: vi.fn(),
  disconnect: vi.fn(),
  showConnect: vi.fn(),
  revokeSession: vi.fn(),
}));

vi.mock('connectkit', () => ({
  ConnectKitButton: {
    Custom: ({ children }: { children: (props: { show: () => void }) => ReactNode }) =>
      children({ show: hoisted.showConnect }),
  },
}));

vi.mock('wagmi', () => ({
  useAccount: () => hoisted.account,
  useDisconnect: () => ({ disconnect: hoisted.disconnect }),
}));

vi.mock('@nanostores/react', () => ({
  useStore: (store: { get?: () => unknown }) => store.get?.() ?? 84532,
}));

vi.mock('@tangle-network/blueprint-ui', () => ({
  cn: (...values: Array<string | false | null | undefined>) => values.filter(Boolean).join(' '),
  publicClient: {
    getBalance: vi.fn(),
  },
  selectedChainIdStore: {
    get: () => 84532,
    set: vi.fn(),
    subscribe: vi.fn(() => () => undefined),
  },
  useWalletEthBalance: () => ({
    balance: '1.42',
  }),
}));

vi.mock('@tangle-network/sandbox-ui/hooks', () => ({
  useDropdownMenu: () => ({
    open: hoisted.dropdownOpen,
    ref: { current: null },
    toggle: hoisted.toggle,
    close: hoisted.close,
  }),
}));

vi.mock('@tangle-network/sandbox-ui/utils', () => ({
  copyText: vi.fn(),
}));

vi.mock('sonner', () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock('~/lib/contracts/chains', () => ({
  networks: {
    84532: {
      chain: {
        id: 84532,
        name: 'Base Sepolia',
        nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
        rpcUrls: { default: { http: ['https://sepolia.base.org'] } },
      },
    },
  },
}));

vi.mock('~/lib/hooks/useOperatorAuth', () => ({
  useOperatorAuth: () => ({ revokeSession: hoisted.revokeSession }),
}));

vi.mock('~/lib/walletRpcSync', () => ({
  expectedLocalRpcUrl: () => 'http://localhost:8645',
  walletRpcMatchesAppRpc: vi.fn(),
}));

describe('WalletButton', () => {
  beforeEach(() => {
    hoisted.account.address = '0x1111111111111111111111111111111111111111';
    hoisted.account.chainId = 84532;
    hoisted.account.isConnected = true;
    hoisted.account.status = 'connected';
    hoisted.dropdownOpen = false;
    hoisted.toggle.mockClear();
    hoisted.close.mockClear();
    hoisted.disconnect.mockClear();
    hoisted.showConnect.mockClear();
    hoisted.revokeSession.mockClear();
  });

  it('keeps compact account placement icon-only while preserving the owner address accessibly', () => {
    render(<WalletButton compact align="start" side="up" />);

    const accountButton = screen.getByRole('button', { name: /account menu 0x1111...1111/i });

    expect(screen.queryByText('0x1111...1111')).not.toBeInTheDocument();
    expect(accountButton).toHaveClass('w-10');
    expect(accountButton).toHaveAttribute('title', '0x1111...1111');
  });

  it('opens the account menu upward from the sidebar with viewport-clamped dimensions', () => {
    hoisted.dropdownOpen = true;

    render(<WalletButton align="start" side="up" />);

    const accountMenu = screen.getByRole('menu', { name: /account actions/i });

    expect(accountMenu).toHaveClass('left-0');
    expect(accountMenu).toHaveClass('bottom-full');
    expect(accountMenu).toHaveClass('mb-2');
    expect(accountMenu).toHaveClass('w-[min(18rem,calc(100vw-1rem))]');
    expect(accountMenu).toHaveClass('max-h-[min(28rem,calc(100vh-1rem))]');
  });

  it('uses an icon-only connect control in collapsed placements', () => {
    hoisted.account.address = undefined;
    hoisted.account.isConnected = false;
    hoisted.account.status = 'disconnected';

    render(<WalletButton compact align="start" side="up" />);

    const connectButton = screen.getByRole('button', { name: /connect wallet/i });

    expect(connectButton).toHaveClass('w-10');
    expect(connectButton).toHaveAttribute('title', 'Connect wallet');
    expect(screen.queryByText('Connect Wallet')).not.toBeInTheDocument();
  });
});
