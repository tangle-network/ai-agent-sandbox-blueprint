import { cn } from '@tangle-network/blueprint-ui';
import { CopyButton } from './CopyButton';

interface LabeledValueRowProps {
  label: string;
  value: string;
  mono?: boolean;
  copyable?: boolean;
  alignRight?: boolean;
}

export function LabeledValueRow({
  label,
  value,
  mono,
  copyable,
  alignRight = false,
}: LabeledValueRowProps) {
  return (
    <div className="flex justify-between text-sm gap-2 group">
      <span className="text-cloud-elements-textSecondary shrink-0">{label}</span>
      <div className="flex items-center gap-1 min-w-0">
        <span className={cn(
          'text-cloud-elements-textPrimary truncate',
          mono && 'font-data text-xs',
          alignRight && 'text-right',
        )}
        >
          {value}
        </span>
        {copyable && <CopyButton value={value} />}
      </div>
    </div>
  );
}
