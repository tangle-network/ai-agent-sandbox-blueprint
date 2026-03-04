import { Button } from '@tangle-network/blueprint-ui/components';

interface SidecarAuthPromptProps {
  message: string;
  hint?: string;
  actionLabel: string;
  busyLabel: string;
  waitingLabel?: string;
  isBusy: boolean;
  isWaiting?: boolean;
  onAuthenticate: () => void;
  buttonVariant?: 'default' | 'secondary';
}

export function SidecarAuthPrompt({
  message,
  hint,
  actionLabel,
  busyLabel,
  waitingLabel,
  isBusy,
  isWaiting,
  onAuthenticate,
  buttonVariant = 'default',
}: SidecarAuthPromptProps) {
  const label = isBusy ? busyLabel : isWaiting ? (waitingLabel ?? actionLabel) : actionLabel;

  return (
    <div className="flex flex-col items-center justify-center h-full gap-3">
      <div className="i-ph:chat-circle text-3xl text-cloud-elements-textTertiary" />
      <p className="text-sm text-cloud-elements-textSecondary">{message}</p>
      {hint && <p className="text-xs text-cloud-elements-textTertiary">{hint}</p>}
      <Button variant={buttonVariant} size="sm" onClick={onAuthenticate} disabled={isBusy || isWaiting}>
        {label}
      </Button>
    </div>
  );
}
