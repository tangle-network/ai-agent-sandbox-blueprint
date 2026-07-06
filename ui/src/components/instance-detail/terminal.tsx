import { Button, Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { OperatorTerminalView } from '~/components/shared/OperatorTerminalView';

interface TerminalTabProps {
  isOperatorAuthed: boolean;
  operatorToken: string | null;
  sshUserDetecting: boolean;
  operatorUrl: string;
  terminalPath: string;
  terminalUsername: string;
  operatorAuthError: string | null;
  handleOperatorAuthenticate: () => void;
  isOperatorAuthenticating: boolean;
  hasWallet: boolean;
}

export function TerminalTab({
  isOperatorAuthed,
  operatorToken,
  sshUserDetecting,
  operatorUrl,
  terminalPath,
  terminalUsername,
  operatorAuthError,
  handleOperatorAuthenticate,
  isOperatorAuthenticating,
  hasWallet,
}: TerminalTabProps) {
  return (
    <Card className="overflow-hidden">
      <CardContent className="p-0">
        {isOperatorAuthed && operatorToken ? (
          sshUserDetecting ? (
            <div className="p-6 text-center">
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                Preparing the sandbox terminal
              </p>
              <p className="text-xs text-cloud-elements-textTertiary">
                Resolving the sandbox user so Terminal starts in the same home directory as SSH.
              </p>
            </div>
          ) : (
            <div className="h-[min(500px,60vh)]">
              <OperatorTerminalView
                apiUrl={operatorUrl}
                resourcePath="/api/sandbox"
                token={operatorToken}
                title="Instance Shell"
                subtitle="Secure shell via operator relay"
                initialCwd={terminalPath}
                displayUsername={terminalUsername}
                displayPath={terminalPath}
              />
            </div>
          )
        ) : (
          <div className="p-6 text-center">
            <p className="text-sm text-cloud-elements-textSecondary mb-3">
              Authenticate with the operator to access the terminal
            </p>
            <p className="text-xs text-cloud-elements-textTertiary mb-4">
              Commands are relayed through the operator API and no longer connect directly to the sandbox container.
            </p>
            {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
            <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
              {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
