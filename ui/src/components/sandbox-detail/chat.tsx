import { Button, Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import type { LocalSandbox } from '~/lib/stores/sandboxes';
import type { ActionTab } from './helpers';

interface ChatTabProps {
  isOperatorAuthed: boolean;
  agentConfigured: boolean;
  agentDiscoveryLoading: boolean;
  hasAgentValidationResult: boolean;
  agentIdentifierValid: boolean;
  configuredAgentIdentifier: string;
  agentAvailableList: string;
  sb: LocalSandbox;
  operatorAuthError: string | null;
  handleOperatorAuthenticate: () => void;
  isOperatorAuthenticating: boolean;
  hasWallet: boolean;
  hasProvisionedSandbox: boolean;
  setTab: (tab: ActionTab) => void;
  agentDiscoveryError: string | null;
  canonicalSandboxId: string | undefined;
  client: SandboxClient | null;
  systemPrompt: string;
  setSystemPrompt: (value: string) => void;
}

export function ChatTab({
  isOperatorAuthed,
  agentConfigured,
  agentDiscoveryLoading,
  hasAgentValidationResult,
  agentIdentifierValid,
  configuredAgentIdentifier,
  agentAvailableList,
  sb,
  operatorAuthError,
  handleOperatorAuthenticate,
  isOperatorAuthenticating,
  hasWallet,
  hasProvisionedSandbox,
  setTab,
  agentDiscoveryError,
  canonicalSandboxId,
  client,
  systemPrompt,
  setSystemPrompt,
}: ChatTabProps) {
  return (
    <Card className="overflow-hidden">
      {!isOperatorAuthed ? (
        <CardContent className="py-16 text-center">
          <div className="i-ph:chat-circle text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
          <p className="text-sm text-cloud-elements-textSecondary mb-2">
            Authenticate with the operator to chat with the sandbox agent
          </p>
          <p className="text-xs text-cloud-elements-textTertiary mb-4">
            Chat requests are proxied through the operator API and no longer connect directly to sandbox containers.
          </p>
          {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
          <Button
            variant="secondary"
            size="sm"
            onClick={handleOperatorAuthenticate}
            disabled={isOperatorAuthenticating || !hasWallet || !hasProvisionedSandbox}
          >
            {isOperatorAuthenticating
              ? 'Signing...'
              : !hasWallet
                ? 'Connect Wallet First'
                : !hasProvisionedSandbox
                  ? 'Waiting for Sandbox...'
                  : 'Authenticate to Chat'}
          </Button>
        </CardContent>
      ) : agentConfigured && agentDiscoveryLoading && !hasAgentValidationResult ? (
        <CardContent className="py-16 text-center">
          <div className="i-ph:spinner-gap text-3xl text-cloud-elements-textTertiary mb-3 mx-auto animate-spin" />
          <p className="text-sm text-cloud-elements-textSecondary">
            Checking which agents this image exposes...
          </p>
        </CardContent>
      ) : agentConfigured && hasAgentValidationResult && !agentIdentifierValid ? (
        <CardContent className="py-16 text-center">
          <div className="i-ph:warning-circle text-3xl text-amber-400 mb-3 mx-auto" />
          <p className="text-sm text-cloud-elements-textSecondary mb-2">
            The configured agent is not available in this sandbox image
          </p>
          <p className="text-xs text-cloud-elements-textTertiary mb-2">
            Configured agent: <span className="font-data">{configuredAgentIdentifier}</span>
          </p>
          <p className="text-xs text-cloud-elements-textTertiary">
            Available agents: <span className="font-data">{agentAvailableList}</span>
          </p>
        </CardContent>
      ) : sb.credentialsAvailable === false ? (
        <CardContent className="py-16 text-center">
          <div className="i-ph:key text-3xl text-amber-400 mb-3 mx-auto" />
          <p className="text-sm text-cloud-elements-textSecondary mb-2">
            AI credentials are not configured
          </p>
          <p className="text-xs text-cloud-elements-textTertiary mb-3">
            Add one of the following in the Secrets tab:
          </p>
          <ul className="text-xs text-cloud-elements-textTertiary space-y-1 mb-4">
            <li><code className="font-data">ANTHROPIC_API_KEY</code></li>
            <li><code className="font-data">ZAI_API_KEY</code></li>
            <li><code className="font-data">OPENCODE_MODEL_PROVIDER</code> + <code className="font-data">OPENCODE_MODEL_NAME</code> + <code className="font-data">OPENCODE_MODEL_API_KEY</code></li>
          </ul>
          <Button size="sm" variant="outline" onClick={() => setTab('secrets')}>
            Go to Secrets
          </Button>
        </CardContent>
      ) : (
        <CardContent className="p-0">
          {agentDiscoveryError && (
            <div className="border-b border-amber-500/20 bg-amber-500/5 px-3 py-2">
              <p className="text-xs text-amber-300">{agentDiscoveryError}</p>
            </div>
          )}
          <div className="h-[min(600px,65vh)]">
            <SessionSidebar
              sandboxId={canonicalSandboxId ?? sb.localId}
              client={client}
              systemPrompt={systemPrompt}
              onSystemPromptChange={setSystemPrompt}
            />
          </div>
        </CardContent>
      )}
    </Card>
  );
}
