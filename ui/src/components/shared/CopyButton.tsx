import { CopyButton as SharedCopyButton } from '@tangle-network/ui/markdown';

interface CopyButtonProps {
  value: string;
  className?: string;
}

export function CopyButton({ value, className }: CopyButtonProps) {
  return (
    <SharedCopyButton
      text={value}
      className={className}
      onClick={(event) => event.stopPropagation()}
    />
  );
}
