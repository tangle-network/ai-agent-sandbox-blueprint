import { cn } from '@tangle-network/blueprint-ui';
import { CopyButton } from './CopyButton';
import { IdentityMark, type IdentityMeta } from './VisualIdentity';

interface LabeledValueRowProps {
  label: string;
  value: string;
  mono?: boolean;
  copyable?: boolean;
  /** Full value to copy when different from the displayed value (e.g. truncated addresses). */
  copyValue?: string;
  alignRight?: boolean;
  identity?: IdentityMeta;
}

export function LabeledValueRow({
  label,
  value,
  mono,
  copyable,
  copyValue,
  alignRight = false,
  identity,
}: LabeledValueRowProps) {
  return (
    <div className="group flex justify-between gap-2 text-sm">
      <span className="flex shrink-0 items-center gap-2 text-cloud-elements-textSecondary">
        {identity ? <IdentityMark identity={identity} size="sm" /> : null}
        <span>{label}</span>
      </span>
      <div className="flex items-center gap-1 min-w-0">
        <span className={cn(
          'text-cloud-elements-textPrimary truncate',
          mono && 'font-data text-xs',
          alignRight && 'text-right',
        )}
        >
          {value}
        </span>
        {copyable && <CopyButton value={copyValue ?? value} />}
      </div>
    </div>
  );
}
