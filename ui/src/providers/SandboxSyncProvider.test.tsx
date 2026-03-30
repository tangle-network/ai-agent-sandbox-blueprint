import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SandboxSyncProvider } from './SandboxSyncProvider';

const {
  sandboxRefreshMock,
  instanceRefreshMock,
  useAccountMock,
  useSandboxHydrationMock,
  useInstanceHydrationMock,
} = vi.hoisted(() => ({
  sandboxRefreshMock: vi.fn(),
  instanceRefreshMock: vi.fn(),
  useAccountMock: vi.fn(),
  useSandboxHydrationMock: vi.fn(),
  useInstanceHydrationMock: vi.fn(),
}));

vi.mock('wagmi', () => ({
  useAccount: () => useAccountMock(),
}));

vi.mock('~/lib/hooks/useSandboxHydration', () => ({
  useSandboxHydration: () => useSandboxHydrationMock(),
}));

vi.mock('~/lib/hooks/useInstanceHydration', () => ({
  useInstanceHydration: () => useInstanceHydrationMock(),
}));

describe('SandboxSyncProvider', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    sandboxRefreshMock.mockReset();
    instanceRefreshMock.mockReset();
    useAccountMock.mockReturnValue({
      address: '0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc',
      isConnected: true,
    });
    useSandboxHydrationMock.mockReturnValue({
      refresh: sandboxRefreshMock,
      authRequired: false,
      isHydrating: false,
      lastError: null,
    });
    useInstanceHydrationMock.mockReturnValue({
      refresh: instanceRefreshMock,
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
      refresh: sandboxRefreshMock,
      authRequired: true,
      isHydrating: false,
      lastError: null,
    });

    render(
      <SandboxSyncProvider>
        <div>child</div>
      </SandboxSyncProvider>,
    );

    expect(sandboxRefreshMock).toHaveBeenCalledWith({ interactive: true });
    expect(instanceRefreshMock).toHaveBeenCalledWith({ interactive: true });
  });

  it('polls passively every five seconds while connected', async () => {
    render(
      <SandboxSyncProvider>
        <div>child</div>
      </SandboxSyncProvider>,
    );

    await vi.advanceTimersByTimeAsync(10_000);

    expect(sandboxRefreshMock).toHaveBeenCalledTimes(2);
    expect(instanceRefreshMock).toHaveBeenCalledTimes(2);
    expect(sandboxRefreshMock).toHaveBeenNthCalledWith(1, { interactive: false });
    expect(sandboxRefreshMock).toHaveBeenNthCalledWith(2, { interactive: false });
    expect(instanceRefreshMock).toHaveBeenNthCalledWith(1, { interactive: false });
    expect(instanceRefreshMock).toHaveBeenNthCalledWith(2, { interactive: false });
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

    expect(sandboxRefreshMock).not.toHaveBeenCalled();
    expect(instanceRefreshMock).not.toHaveBeenCalled();
  });
});
