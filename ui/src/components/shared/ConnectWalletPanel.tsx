import { ConnectKitButton } from 'connectkit';

interface ConnectWalletPanelProps {
  // Optional override for the headline; defaults to the deploy-flow message.
  title?: string;
  // Optional override for the one-line explainer.
  description?: string;
}

// Theme-aware empty state shown when the iframe loads without a connected
// wallet. Uses the same cloud-elements design tokens as the rest of the app
// so it reads cleanly on both dark and light themes — no hardcoded colors.
export function ConnectWalletPanel({
  title = 'Connect your wallet to continue',
  description = 'Provisioning and managing sandboxes requires a connected wallet on Tangle Network.',
}: ConnectWalletPanelProps) {
  return (
    <div className="sandbox-console-panel flex flex-col gap-3 rounded-md p-4 sm:flex-row sm:items-center sm:justify-between">
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] text-[var(--sandbox-console-muted)]">
          <span className="i-ph:wallet text-lg" />
        </div>
        <div className="min-w-0">
          <p className="font-display text-sm font-semibold text-[var(--sandbox-console-text)]">{title}</p>
          <p className="mt-0.5 max-w-2xl text-xs leading-5 text-[var(--sandbox-console-muted)]">{description}</p>
        </div>
      </div>
      <ConnectKitButton.Custom>
        {({ show, isConnecting }) => (
          <button
            type="button"
            onClick={show}
            disabled={isConnecting}
            className="inline-flex h-10 shrink-0 items-center justify-center gap-2 rounded-md border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] px-4 font-display text-sm font-semibold text-[var(--sandbox-console-text)] transition-colors hover:border-[var(--sandbox-console-brand)] hover:bg-[rgba(142,89,255,0.22)] disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isConnecting ? (
              <>
                <span className="h-3 w-3 animate-spin rounded-full border-2 border-[rgba(142,89,255,0.35)] border-t-[var(--sandbox-console-brand)]" />
                Connecting
              </>
            ) : (
              <>
                <span className="i-ph:plugs-connected text-base" />
                Connect Wallet
              </>
            )}
          </button>
        )}
      </ConnectKitButton.Custom>
    </div>
  );
}
