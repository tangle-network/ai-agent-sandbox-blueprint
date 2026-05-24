import { ConnectKitButton } from 'connectkit';
import { Card, CardContent } from '@tangle-network/blueprint-ui/components';

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
    <Card>
      <CardContent className="p-6">
        <div className="py-10 text-center">
          <div className="i-ph:wallet text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
          <p className="text-base font-display font-semibold text-cloud-elements-textPrimary">
            {title}
          </p>
          <p className="text-sm text-cloud-elements-textSecondary mt-1 max-w-md mx-auto">
            {description}
          </p>
          <div className="mt-5 inline-flex">
            <ConnectKitButton.Custom>
              {({ show, isConnecting }) => (
                <button
                  type="button"
                  onClick={show}
                  disabled={isConnecting}
                  className="px-4 py-2.5 rounded-lg bg-violet-500/10 border border-violet-500/20 text-violet-700 dark:text-violet-400 text-sm font-display font-medium hover:bg-violet-500/20 disabled:opacity-60 disabled:cursor-not-allowed transition-colors"
                >
                  {isConnecting ? (
                    <span className="flex items-center gap-2">
                      <span className="w-3 h-3 rounded-full border-2 border-violet-500/40 border-t-violet-600 dark:border-t-violet-400 animate-spin" />
                      Connecting...
                    </span>
                  ) : (
                    <span className="flex items-center gap-2">
                      <span className="i-ph:plugs-connected text-base" />
                      Connect Wallet
                    </span>
                  )}
                </button>
              )}
            </ConnectKitButton.Custom>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
