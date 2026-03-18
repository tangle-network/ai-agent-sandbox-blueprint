# @tangle-network/agent-ui (DEPRECATED)

> **This package is deprecated.** Use the following replacements:
>
> - **General UI components** (chat, terminal, primitives, dashboard, etc.) → [`@tangle-network/sandbox-ui`](https://www.npmjs.com/package/@tangle-network/sandbox-ui)
> - **Web3/wagmi components** (ConnectWalletCta, useWagmiSidecarAuth, useWalletEthBalance) → [`@tangle-network/blueprint-ui`](https://github.com/tangle-network/blueprint-ui)

## Migration

```diff
- import { ChatContainer } from '@tangle-network/agent-ui';
+ import { ChatContainer } from '@tangle-network/sandbox-ui/chat';

- import { useWagmiSidecarAuth } from '@tangle-network/agent-ui';
+ import { useWagmiSidecarAuth } from '@tangle-network/blueprint-ui';

- import { copyText, timeAgo } from '@tangle-network/agent-ui/primitives';
+ import { cn } from '@tangle-network/sandbox-ui/utils';
```
