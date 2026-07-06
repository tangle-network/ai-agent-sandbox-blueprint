import { Button, Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { OperatorTerminalView } from '~/components/shared/OperatorTerminalView';

interface TerminalTabProps {
  isOperatorAuthed: boolean;
  operatorToken: string | null;
  operatorAuthError: string | null;
  onAuthenticate: () => void;
  isOperatorAuthenticating: boolean;
  hasWallet: boolean;
  hasProvisionedSandbox: boolean;
  sshUserDetecting: boolean;
  operatorUrl: string;
  operatorResourcePath: string;
  terminalPath: string;
  terminalUsername: string;
}

export function TerminalTab({
  isOperatorAuthed,
  operatorToken,
  operatorAuthError,
  onAuthenticate,
  isOperatorAuthenticating,
  hasWallet,
  hasProvisionedSandbox,
  sshUserDetecting,
  operatorUrl,
  operatorResourcePath,
  terminalPath,
  terminalUsername,
}: TerminalTabProps) {
  return (
    <Card className="overflow-hidden">
      {!isOperatorAuthed || !operatorToken ? (
        <CardContent className="py-16 text-center">
          <div className="i-ph:terminal-window text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
          <p className="text-sm text-cloud-elements-textSecondary mb-2">
            Authenticate with the operator to access the sandbox terminal
          </p>
          <p className="text-xs text-cloud-elements-textTertiary mb-4">
            The browser talks only to the operator API, which verifies sandbox ownership before relaying commands.
          </p>
          {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
          <Button
            variant="secondary"
            size="sm"
            onClick={onAuthenticate}
            disabled={isOperatorAuthenticating || !hasWallet || !hasProvisionedSandbox}
          >
            {isOperatorAuthenticating
              ? 'Signing...'
              : !hasWallet
                ? 'Connect Wallet First'
                : !hasProvisionedSandbox
                  ? 'Waiting for Sandbox...'
                  : 'Connect Terminal'}
          </Button>
        </CardContent>
      ) : sshUserDetecting ? (
        <CardContent className="py-16 text-center">
          <div className="i-ph:terminal-window text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
          <p className="text-sm text-cloud-elements-textSecondary mb-2">
            Preparing the sandbox terminal
          </p>
          <p className="text-xs text-cloud-elements-textTertiary">
            Resolving the sandbox user so Terminal starts in the same home directory as SSH.
          </p>
        </CardContent>
      ) : (
        <CardContent className="p-0">
          <div className="h-[min(500px,60vh)]">
            <OperatorTerminalView
              apiUrl={operatorUrl}
              resourcePath={operatorResourcePath}
              token={operatorToken}
              title="Sandbox Shell"
              subtitle="Secure shell via operator relay"
              initialCwd={terminalPath}
              displayUsername={terminalUsername}
              displayPath={terminalPath}
            />
          </div>
        </CardContent>
      )}
    </Card>
  );
}
