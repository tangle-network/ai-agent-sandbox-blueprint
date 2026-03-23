---
name: wallet-inject
description: >
  Inject a mock Web3 wallet (EIP-1193 + EIP-6963) into Playwright pages for E2E
  testing against the Sandbox Cloud UI. Use this skill whenever you need to write
  or run Playwright tests that require a connected wallet, interact with the UI
  via Playwright MCP with wallet connectivity, or debug wallet-related E2E
  failures. Also use it when the user mentions mock wallet, wallet injection,
  ConnectKit testing, wagmi E2E, or wants to automate any UI flow that normally
  requires MetaMask or another browser wallet.
---

# Web3 Wallet Injection for Playwright

This skill provides the pattern for injecting a mock EIP-1193 wallet provider
into Playwright pages so E2E tests (and interactive Playwright MCP sessions) can
interact with the Sandbox Cloud UI as if a real wallet is connected — no MetaMask
extension, no manual approval popups.

## How it works

The UI uses **wagmi + ConnectKit**. The mock provider:

1. Sets `window.ethereum` with `isMetaMask: true` so ConnectKit recognizes it
2. Announces itself via **EIP-6963** so wagmi auto-discovers it
3. Returns a fixed Anvil account for `eth_accounts` / `eth_requestAccounts`
4. Proxies all other JSON-RPC calls to the local Anvil node (port `8645`), which
   has accounts unlocked — no private-key signing in the browser

## Prerequisites

Before running wallet-connected E2E tests, the local stack must be up:

```bash
SKIP_BUILD=1 ./scripts/deploy-local.sh   # Anvil + operators + .env.local
pnpm --dir ui dev                         # dev server on :1338
```

Playwright must be installed: `pnpm --dir ui exec playwright install chromium`

## The injection helper

Create `ui/e2e/wallet-inject.ts` (if it doesn't already exist) with this content:

```typescript
import type { Page } from '@playwright/test';

/** Anvil default account #0 */
export const MOCK_ACCOUNT = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266' as const;
export const MOCK_CHAIN_ID = 31337;
export const MOCK_CHAIN_ID_HEX = '0x7a69';
export const ANVIL_RPC_URL = 'http://127.0.0.1:8645';

/**
 * Inject a mock EIP-1193 wallet provider into the page.
 * Must be called BEFORE page.goto().
 */
export async function injectMockWallet(
  page: Page,
  opts: {
    account?: string;
    chainId?: number;
    rpcUrl?: string;
  } = {},
) {
  const account = opts.account ?? MOCK_ACCOUNT;
  const chainId = opts.chainId ?? MOCK_CHAIN_ID;
  const chainIdHex = '0x' + chainId.toString(16);
  const rpcUrl = opts.rpcUrl ?? ANVIL_RPC_URL;

  await page.addInitScript(`
    (function() {
      const ACCOUNT = '${account}';
      const CHAIN_ID_HEX = '${chainIdHex}';
      const CHAIN_ID_DEC = '${chainId}';
      const RPC_URL = '${rpcUrl}';
      let currentChainId = CHAIN_ID_HEX;

      const listeners = {};
      function on(event, fn) {
        (listeners[event] = listeners[event] || []).push(fn);
      }
      function removeListener(event, fn) {
        if (listeners[event]) {
          listeners[event] = listeners[event].filter(f => f !== fn);
        }
      }
      function emit(event, ...args) {
        (listeners[event] || []).forEach(fn => {
          try { fn(...args); } catch(e) { console.error('[mock-wallet] emit error:', e); }
        });
      }

      let rpcId = 1;
      async function proxyToRpc(method, params) {
        const res = await fetch(RPC_URL, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: rpcId++, method, params }),
        });
        const json = await res.json();
        if (json.error) throw new Error(json.error.message || JSON.stringify(json.error));
        return json.result;
      }

      const provider = {
        isMetaMask: true,
        isConnected: () => true,
        chainId: CHAIN_ID_HEX,
        networkVersion: CHAIN_ID_DEC,
        selectedAddress: ACCOUNT,
        on,
        addListener: on,
        removeListener,
        removeAllListeners: (event) => {
          if (event) delete listeners[event];
          else Object.keys(listeners).forEach(k => delete listeners[k]);
        },
        once: (event, fn) => {
          const wrapped = (...args) => { removeListener(event, wrapped); fn(...args); };
          on(event, wrapped);
        },
        emit,
        request: async ({ method, params }) => {
          switch (method) {
            case 'eth_chainId':
              return currentChainId;
            case 'net_version':
              return CHAIN_ID_DEC;
            case 'eth_accounts':
            case 'eth_requestAccounts':
              return [ACCOUNT];
            case 'wallet_requestPermissions':
              return [{ parentCapability: 'eth_accounts' }];
            case 'wallet_switchEthereumChain': {
              const newChainId = params?.[0]?.chainId;
              if (newChainId) {
                currentChainId = newChainId;
                provider.chainId = newChainId;
                emit('chainChanged', newChainId);
              }
              return null;
            }
            case 'wallet_addEthereumChain':
              return null;
            default:
              return await proxyToRpc(method, params);
          }
        },
      };

      window.ethereum = provider;

      const info = Object.freeze({
        uuid: 'e2e-mock-wallet-0001',
        name: 'E2E Mock Wallet',
        icon: 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle cx="16" cy="16" r="14" fill="%234F46E5"/></svg>',
        rdns: 'io.tangle.e2e-mock',
      });
      const detail = Object.freeze({ info, provider });

      window.addEventListener('eip6963:requestProvider', () => {
        window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail }));
      });
      setTimeout(() => {
        window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail }));
      }, 0);

      console.log('[mock-wallet] Injected for', ACCOUNT, 'on chain', CHAIN_ID_HEX);
    })();
  `);
}
```

## Usage patterns

### In a Playwright test

```typescript
import { test, expect } from './fixtures.js';
import { injectMockWallet, MOCK_ACCOUNT } from './wallet-inject.js';

test('dashboard shows connected wallet', async ({ page }) => {
  await injectMockWallet(page);
  await page.goto('/');
  await expect(page.locator(`text=0xf39F`)).toBeVisible();
});
```

### With a different Anvil account

Anvil ships with 10 pre-funded accounts. Pass a different index if you need
multiple wallets in the same test:

```typescript
await injectMockWallet(page, {
  account: '0x70997970C51812dc3A010C7d01b50e0d17dc79C8', // account #1
});
```

### Auto-inject via fixtures

Extend the shared fixtures so every test gets the wallet automatically:

```typescript
// e2e/fixtures.ts (extended)
import { test as base } from '@playwright/test';
import { injectMockWallet } from './wallet-inject.js';

export const test = base.extend<{ /* ...existing types */ }>({
  page: async ({ page }, use) => {
    await injectMockWallet(page);
    await use(page);
  },
  // ...existing fixtures...
});
```

### Via Playwright MCP (interactive)

When using Playwright MCP tools (`browser_run_code`, `browser_navigate`, etc.),
inject the wallet before navigating:

```javascript
// browser_run_code payload
async (page) => {
  await page.addInitScript(`
    (function() {
      // ... full injection script from above ...
    })();
  `);
  await page.goto('http://localhost:1338', { waitUntil: 'domcontentloaded' });
  return 'Wallet injected';
}
```

`addInitScript` must be called **before** `page.goto()`. If the page is already
loaded, call `page.goto()` again after adding the script — it re-executes on
every navigation.

## Anvil default accounts

| Index | Address |
|-------|---------|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` |
| 2 | `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC` |

These are well-known test keys. Never use them on a real network.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Wallet not detected by ConnectKit | Ensure `addInitScript` runs before `page.goto()` |
| `ERR_CONNECTION_REFUSED` on RPC | Start Anvil with `SKIP_BUILD=1 ./scripts/deploy-local.sh` |
| Transaction fails "nonce too high" | Reload the page — Anvil was likely restarted |
| Balance shows 0 ETH | Check Anvil is on port 8645 and chain ID is 31337 |
