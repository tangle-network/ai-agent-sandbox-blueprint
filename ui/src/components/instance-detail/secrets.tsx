import type { Dispatch, SetStateAction } from 'react';
import { Button, Card, CardContent, CardDescription, CardHeader, CardTitle, Textarea } from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';

interface SecretsTabProps {
  isOperatorAuthed: boolean;
  secretsVisible: boolean;
  setSecretsVisible: Dispatch<SetStateAction<boolean>>;
  secretsLoading: boolean;
  secretsJson: string;
  setSecretsJson: (value: string) => void;
  secretsError: string | null;
  secretsSuccess: string | null;
  handleInjectSecrets: () => void;
  secretsBusy: boolean;
  handleWipeSecrets: () => void;
  operatorAuthError: string | null;
  handleOperatorAuthenticate: () => void;
  isOperatorAuthenticating: boolean;
  hasWallet: boolean;
}

export function SecretsTab({
  isOperatorAuthed,
  secretsVisible,
  setSecretsVisible,
  secretsLoading,
  secretsJson,
  setSecretsJson,
  secretsError,
  secretsSuccess,
  handleInjectSecrets,
  secretsBusy,
  handleWipeSecrets,
  operatorAuthError,
  handleOperatorAuthenticate,
  isOperatorAuthenticating,
  hasWallet,
}: SecretsTabProps) {
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Environment Secrets</CardTitle>
          <CardDescription>Inject environment variables as secrets into the instance</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {isOperatorAuthed ? (
            <>
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-medium text-cloud-elements-textSecondary" htmlFor="instance-secrets-json">
                    Secrets (JSON object)
                  </label>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 w-7 p-0"
                    onClick={() => setSecretsVisible((v) => !v)}
                    title={secretsVisible ? 'Hide secrets' : 'Show secrets'}
                  >
                    <div className={cn('text-sm', secretsVisible ? 'i-ph:eye' : 'i-ph:eye-slash')} />
                  </Button>
                </div>
                {secretsLoading && (
                  <p className="text-xs text-cloud-elements-textTertiary">Loading existing secrets...</p>
                )}
                <Textarea
                  id="instance-secrets-json"
                  value={secretsJson}
                  onChange={(e) => setSecretsJson(e.target.value)}
                  placeholder='{"API_KEY": "sk-...", "DB_URL": "postgres://..."}'
                  className="font-data text-xs min-h-[120px] resize-y"
                  style={{ filter: secretsVisible ? 'none' : 'blur(4px)' }}
                  disabled={secretsLoading}
                />
                <p className="text-[11px] text-cloud-elements-textTertiary">
                  Key-value pairs injected as environment variables. Injecting replaces all existing secrets. Values are encrypted at rest.
                </p>
              </div>
              {secretsError && (
                <p className="text-xs text-red-400">{secretsError}</p>
              )}
              {secretsSuccess && (
                <p className="text-xs text-teal-400">{secretsSuccess}</p>
              )}
              <div className="flex items-center gap-2">
                <Button size="sm" onClick={handleInjectSecrets} disabled={secretsBusy || secretsLoading}>
                  {secretsBusy ? 'Injecting...' : 'Inject Secrets'}
                </Button>
                <Button variant="destructive" size="sm" onClick={handleWipeSecrets} disabled={secretsBusy || secretsLoading}>
                  Wipe All Secrets
                </Button>
              </div>
            </>
          ) : (
            <div className="p-2 text-center">
              <p className="text-sm text-cloud-elements-textSecondary mb-3">
                Authenticate with the operator to manage instance secrets
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-4">
                Secret updates are proxied through the operator API and may restart the instance sidecar to apply changes.
              </p>
              {operatorAuthError && <p className="text-xs text-crimson-500 mb-4">{operatorAuthError}</p>}
              <Button size="sm" onClick={handleOperatorAuthenticate} disabled={isOperatorAuthenticating || !hasWallet}>
                {isOperatorAuthenticating ? 'Signing...' : !hasWallet ? 'Connect Wallet First' : 'Authenticate'}
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
