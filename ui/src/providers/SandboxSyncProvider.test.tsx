import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SandboxSyncProvider } from './SandboxSyncProvider';

const { refreshMock, useAccountMock, useSandboxHydrationMock } = vi.hoisted(() => ({
  refreshMock: vi.fn(),
  useAccountMock: vi.fn(),
  useSandboxHydrationMock: vi.fn(),
}));

vi.mock('wagmi', () => ({
  useAccount: () => useAccountMock(),
}));

vi.mock('~/lib/hooks/useSandboxHydration', () => ({
  useSandboxHydration: () => useSandboxHydrationMock(),
}));

describe('SandboxSyncProvider', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    refreshMock.mockReset();
    useAccountMock.mockReturnValue({
      address: '0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc',
      isConnected: true,
    });
    useSandboxHydrationMock.mockReturnValue({
      refresh: refreshMock,
      authRequired: false,
      isHydrating: false,
      lastError: null,
    });
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      value: 'visible',
    });
  });

  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it('attempts interactive operator auth once when passive hydration reports authRequired', () => {
    useSandboxHydrationMock.mockReturnValue({
      refresh: refreshMock,
      authRequired: true,
      isHydrating: false,
      lastError: null,
    });

    render(
      <SandboxSyncProvider>
        <div>child</div>
      </SandboxSyncProvider>,
    );

    expect(refreshMock).toHaveBeenCalledWith({ interactive: true });
  });

  it('polls passively every five seconds while connected', async () => {
    render(
      <SandboxSyncProvider>
        <div>child</div>
      </SandboxSyncProvider>,
    );

    await vi.advanceTimersByTimeAsync(10_000);

    expect(refreshMock).toHaveBeenCalledTimes(2);
    expect(refreshMock).toHaveBeenNthCalledWith(1, { interactive: false });
    expect(refreshMock).toHaveBeenNthCalledWith(2, { interactive: false });
  });

  it('skips passive polls while the document is hidden', async () => {
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      value: 'hidden',
    });

    render(
      <SandboxSyncProvider>
        <div>child</div>
      </SandboxSyncProvider>,
    );

    await vi.advanceTimersByTimeAsync(10_000);

    expect(refreshMock).not.toHaveBeenCalled();
  });
});
