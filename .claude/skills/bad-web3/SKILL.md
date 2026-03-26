---
name: bad-web3
description: "Run the `bad` CLI (Browser Agent Driver) against the local frontend with a web3 wallet (MetaMask) connected. Use this skill when the user wants to browse, test, or interact with the local UI at localhost:1338 using a connected wallet via `bad`. Triggers on: bad + wallet, bad + web3, bad + MetaMask, browse frontend with wallet, test UI with wallet connected."
---

Run `bad` CLI with a real MetaMask wallet extension against the local frontend.

## Provider

Default: `--provider claude-code --model sonnet` (uses local `claude` CLI OAuth session, no API key needed).

Only use a different provider if the user explicitly asks. Available alternatives:

| Provider | Flags | Auth |
|----------|-------|------|
| `claude-code` (default) | `--provider claude-code --model sonnet` | `claude login` OAuth |
| `codex-cli` | `--provider codex-cli --model gpt-5` | `codex login` OAuth |
| `anthropic` | `--provider anthropic --model claude-sonnet-4-6` | `ANTHROPIC_API_KEY` env var |
| `openai` | `--provider openai --model gpt-5.4` | `OPENAI_API_KEY` env var |

## Prerequisites (check before running)

Run these checks before executing `bad`:

1. **Frontend running**: `curl -s -o /dev/null -w '%{http_code}' http://localhost:1338` — must return 200. If not, tell the user to start it with `pnpm --dir ui dev`.
2. **`bad` CLI available**: `which bad` — must resolve. If not, the user needs to install or build browser-agent-driver.
3. **MetaMask extension**: Find the browser-agent-driver project (check `~/development/tangle/browser-agent-driver` or resolve from `which bad`), then check `extensions/metamask` exists. If missing, run `pnpm wallet:setup` in that project.
4. **Wallet profile**: Check `.agent-wallet-profile` exists in the browser-agent-driver project. If missing, run `pnpm wallet:onboard` in that project.

## Pre-authenticate Operator API

MetaMask 13.x `personal_sign` popups are not reliably auto-approved in Playwright. To work around this, pre-generate operator API auth tokens via shell commands and inject them into the `bad` goal.

Run this **before** launching `bad`. Determine the wallet address and private key from the Anvil Accounts table below (default: account #0).

```bash
WALLET_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
WALLET_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

# Read operator URLs from ui/.env.local
OPERATOR_URL=$(grep '^VITE_OPERATOR_API_URL=' ui/.env.local | cut -d= -f2-)
INSTANCE_OPERATOR_URL=$(grep '^VITE_INSTANCE_OPERATOR_API_URL=' ui/.env.local | cut -d= -f2-)
```

Then for each operator URL, get a PASETO token:

```bash
# IMPORTANT: use printf, NOT echo — zsh echo breaks JSON newlines
CHALLENGE=$(curl -s -X POST "${OPERATOR_URL}/api/auth/challenge" \
  -H 'Content-Type: application/json' \
  -d "{\"address\":\"${WALLET_ADDR}\"}")
NONCE=$(printf '%s' "$CHALLENGE" | jq -r '.nonce')
EXPIRES=$(printf '%s' "$CHALLENGE" | jq -r '.expires_at')

MESSAGE="Sign this message to authenticate with Tangle Sandbox.

Nonce: ${NONCE}
Expires: ${EXPIRES}"
SIGNATURE=$(cast wallet sign --private-key "$WALLET_KEY" "$MESSAGE")

SESSION=$(curl -s -X POST "${OPERATOR_URL}/api/auth/session" \
  -H 'Content-Type: application/json' \
  -d "{\"nonce\":\"${NONCE}\",\"signature\":\"${SIGNATURE}\"}")
TOKEN=$(printf '%s' "$SESSION" | jq -r '.token')
TOKEN_EXPIRES=$(printf '%s' "$SESSION" | jq -r '.expires_at')
```

**Validate**: `TOKEN` must start with `v4.local.`. If pre-auth fails (e.g. `cast` not installed, operator down), skip it and warn the user — the run will proceed without pre-auth but operator-authenticated features (sandbox list, chat, terminal) will not work.

Repeat for `INSTANCE_OPERATOR_URL` if it exists (store as `INSTANCE_TOKEN` / `INSTANCE_TOKEN_EXPIRES`).

### Inject tokens into the goal

Prepend a `runScript` injection step to the user's goal. The sessionStorage key format is `tangle.operator_auth.{address_lowercase}::{operator_url}`.

Build the injection script string (example for account #0 with both operators):

```
BEFORE doing anything else, run this script to inject operator auth tokens:
sessionStorage.setItem('tangle.operator_auth.0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266::http://127.0.0.1:9102', JSON.stringify({token:"<SANDBOX_TOKEN>",expiresAt:<SANDBOX_TOKEN_EXPIRES>}));
sessionStorage.setItem('tangle.operator_auth.0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266::http://127.0.0.1:9202', JSON.stringify({token:"<INSTANCE_TOKEN>",expiresAt:<INSTANCE_TOKEN_EXPIRES>}));
location.reload();
Then wait 3 seconds for the page to reload with auth active. After that proceed with the actual goal: <USER_GOAL>
```

Replace `<SANDBOX_TOKEN>`, `<SANDBOX_TOKEN_EXPIRES>`, `<INSTANCE_TOKEN>`, `<INSTANCE_TOKEN_EXPIRES>` with the actual values from the pre-auth step. The address must be fully lowercase. Use the actual operator URLs from `ui/.env.local`.

## Command Pattern

Resolve `<BAD_PROJECT>` to the browser-agent-driver project directory (e.g. from `which bad` or common locations like `~/development/tangle/browser-agent-driver`).

```bash
bad run \
  --wallet \
  --wallet-auto-approve \
  --wallet-chain-id 31337 \
  --wallet-chain-rpc-url http://127.0.0.1:8645 \
  --wallet-seed-url http://localhost:1338 \
  --extension <BAD_PROJECT>/extensions/metamask \
  --user-data-dir <BAD_PROJECT>/.agent-wallet-profile \
  --no-headless \
  --no-memory \
  --provider claude-code --model sonnet \
  --goal "<GOAL_WITH_INJECTION_PREFIX>" \
  --url http://localhost:1338 \
  --max-turns 15 \
  --debug
```

Replace `<BAD_PROJECT>` and `<GOAL_WITH_INJECTION_PREFIX>` (the injection prefix + user goal). Only change the provider/model if the user explicitly asks for a different one.

## Output Mode

By default, use `--no-memory` and no `--sink` (results are discarded). The user may override:

- **"save to tmp"** — add `--sink /tmp/bad-web3-results` and remove `--no-memory`
- **"save to project"** / **"save results"** — add `--sink ./agent-results` and remove `--no-memory`
- **"no memory"** (default) — keep `--no-memory`, no `--sink`

## Anvil Accounts

The default MetaMask profile uses account #0. If the user specifies a different account, mention the address for context. All accounts share the same seed phrase: `test test test test test test test test test test test junk`

| # | Address | Private Key |
|---|---------|-------------|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |
| 2 | `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC` | `0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a` |
| 3 | `0x90F79bf6EB2c4f870365E785982E1f101E93b906` | `0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6` |
| 4 | `0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65` | `0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a` |
| 5 | `0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc` | `0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba` |
| 6 | `0x976EA74026E726554dB657fA54763abd0C3a0aa9` | `0x92db14e403b83dfe3df233f83dfa3ecda7b66277160eb78dab8c29d8e9a9d8b5` |
| 7 | `0x14dC79964da2C08dA15Fd353d30d9CBa9C4a9F8a` | `0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356` |
| 8 | `0x23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f` | `0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97` |
| 9 | `0xa0Ee7A142d267C1f36714E4a8F75612F20a79720` | `0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6` |

- Chain: 31337 (local Anvil)
- RPC: `http://127.0.0.1:8645`
- MetaMask password: `TangleLocal123!`

> These are well-known test keys. **Never** use them on a real network.

## Notes

- Always use `--no-headless` for wallet mode (MetaMask extension requires a visible browser).
- The `--wallet-auto-approve` flag handles MetaMask popups automatically (wallet connect, chain switch, transactions). However, `personal_sign` popups may not be reliably auto-approved in MetaMask 13.x — use the pre-authentication flow above to bypass this.
- Run the command in the background with a timeout of 300000ms since it takes time for the agent to navigate.
- To find the browser-agent-driver project: parse the path from `which bad` (reads the shell script shebang), or check common locations like `~/development/tangle/browser-agent-driver`.
- Use `printf '%s'` (not `echo`) when piping API responses to `jq` — zsh's `echo` interprets escape sequences in JSON, breaking the parser.
