import type { Dispatch, SetStateAction } from 'react';
import { Button, Card, CardContent, CardDescription, CardHeader, CardTitle, Textarea } from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';

interface SecretsTabProps {
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
}

export function SecretsTab({
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
}: SecretsTabProps) {
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Environment Secrets</CardTitle>
          <CardDescription>Inject environment variables as secrets into the sandbox</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-medium text-cloud-elements-textSecondary">
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
            <Button
              size="sm"
              onClick={handleInjectSecrets}
              disabled={secretsBusy || secretsLoading}
            >
              {secretsBusy ? 'Injecting...' : 'Inject Secrets'}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={handleWipeSecrets}
              disabled={secretsBusy || secretsLoading}
            >
              Wipe All Secrets
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
