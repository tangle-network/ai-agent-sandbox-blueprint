import { Button, Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { SessionSidebar } from '~/components/shared/SessionSidebar';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import type { LocalInstance } from '~/lib/stores/instances';
import type { ActionTab } from './helpers';

interface ChatTabProps {
  isOperatorAuthed: boolean;
  agentConfigured: boolean;
  agentDiscoveryLoading: boolean;
  hasAgentValidationResult: boolean;
  agentIdentifierValid: boolean;
  configuredAgentIdentifier: string;
  agentAvailableList: string;
  inst: LocalInstance;
  operatorAuthError: string | null;
  handleOperatorAuthenticate: () => void;
  isOperatorAuthenticating: boolean;
  hasWallet: boolean;
  setTab: (tab: ActionTab) => void;
  agentDiscoveryError: string | null;
  decodedId: string;
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
  inst,
  operatorAuthError,
  handleOperatorAuthenticate,
  isOperatorAuthenticating,
  hasWallet,
  setTab,
  agentDiscoveryError,
  decodedId,
  client,
  systemPrompt,
  setSystemPrompt,
}: ChatTabProps) {
  return (
    <Card className="overflow-hidden">
      {!isOperatorAuthed ? (
        <CardContent className="p-0">
          <div className="p-6 text-center">
            <p className="text-sm text-cloud-elements-textSecondary mb-3">
              Authenticate with the operator to chat with the instance agent
            </p>
            <p className="text-xs text-cloud-elements-textTertiary mb-4">
              Chat requests are proxied through the operator API and do not expose the sandbox container to the browser.
            </p>
            {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
            <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
              {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
            </Button>
          </div>
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
            The configured agent is not available in this instance image
          </p>
          <p className="text-xs text-cloud-elements-textTertiary mb-2">
            Configured agent: <span className="font-data">{configuredAgentIdentifier}</span>
          </p>
          <p className="text-xs text-cloud-elements-textTertiary">
            Available agents: <span className="font-data">{agentAvailableList}</span>
          </p>
        </CardContent>
      ) : inst.credentialsAvailable === false ? (
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
              sandboxId={inst.sandboxId ?? decodedId}
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
