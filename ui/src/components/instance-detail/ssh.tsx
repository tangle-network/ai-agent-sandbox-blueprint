import { Button, Card, CardContent, CardDescription, CardHeader, CardTitle, Input, Textarea } from '@tangle-network/blueprint-ui/components';
import type { SshKey } from './helpers';

interface SshTabProps {
  sshConnectionCommand: string;
  sshUsername: string;
  setSshUsername: (value: string) => void;
  sshUsernameDirtyRef: { current: boolean };
  sshUserDetecting: boolean;
  sshUserHint: string | null;
  sshPublicKey: string;
  setSshPublicKey: (value: string) => void;
  sshError: string | null;
  sshSuccess: string | null;
  handleSshProvision: () => void;
  sshBusy: boolean;
  sshKeys: SshKey[];
  handleSshRevoke: (key: SshKey) => void;
}

export function SshTab({
  sshConnectionCommand,
  sshUsername,
  setSshUsername,
  sshUsernameDirtyRef,
  sshUserDetecting,
  sshUserHint,
  sshPublicKey,
  setSshPublicKey,
  sshError,
  sshSuccess,
  handleSshProvision,
  sshBusy,
  sshKeys,
  handleSshRevoke,
}: SshTabProps) {
  return (
    <div className="space-y-4">
      {sshConnectionCommand && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">SSH Connection</CardTitle>
            <CardDescription>
              Connect to this instance via SSH using the command below.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-2">
            <p className="text-xs font-data rounded-lg bg-cloud-elements-background-depth-2 px-3 py-2 break-all">
              {sshConnectionCommand}
            </p>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Add SSH Key</CardTitle>
          <CardDescription>Provision an SSH public key for remote access</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-cloud-elements-textSecondary">Username</label>
            <Input
              aria-label="SSH username"
              value={sshUsername}
              onChange={(e) => {
                sshUsernameDirtyRef.current = true;
                setSshUsername(e.target.value);
              }}
              placeholder={sshUserDetecting ? 'Detecting sandbox user...' : 'Auto-detected from sandbox'}
              className="font-data text-sm"
            />
            {sshUserHint && (
              <p className="text-xs text-cloud-elements-textSecondary">{sshUserHint}</p>
            )}
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-cloud-elements-textSecondary">Public Key</label>
            <Textarea
              aria-label="SSH public key"
              value={sshPublicKey}
              onChange={(e) => setSshPublicKey(e.target.value)}
              placeholder="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5..."
              className="font-data text-xs min-h-[80px] resize-none"
            />
          </div>
          {sshError && (
            <p className="text-xs text-red-400">{sshError}</p>
          )}
          {sshSuccess && (
            <p className="text-xs text-teal-400">{sshSuccess}</p>
          )}
          <Button
            size="sm"
            onClick={handleSshProvision}
            disabled={sshBusy || !sshPublicKey.trim()}
          >
            {sshBusy ? 'Provisioning...' : 'Add Key'}
          </Button>
        </CardContent>
      </Card>

      {sshKeys.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Active Keys</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {sshKeys.map((key) => (
              <div
                key={key.publicKey}
                className="flex items-center justify-between gap-3 p-3 rounded-lg bg-cloud-elements-background-depth-2"
              >
                <div className="min-w-0">
                  <span className="text-xs font-data text-cloud-elements-textSecondary">{key.username}@</span>
                  <span className="text-xs font-data text-cloud-elements-textTertiary truncate block">
                    {key.publicKey.length > 60 ? `${key.publicKey.slice(0, 60)}...` : key.publicKey}
                  </span>
                </div>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => handleSshRevoke(key)}
                  disabled={sshBusy}
                >
                  Revoke
                </Button>
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
