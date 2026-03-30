import { cn } from '@tangle-network/blueprint-ui';
import { StatusBadge } from './StatusBadge';

interface ResourceIdentityProps {
  name: string;
  status: string;
  statusLabel?: string;
  teeEnabled?: boolean;
  image: string;
  specs: string;
  titleClassName?: string;
  teeStyle?: 'pill' | 'text';
}

export function ResourceIdentity({
  name,
  status,
  statusLabel,
  teeEnabled,
  image,
  specs,
  titleClassName,
  teeStyle = 'pill',
}: ResourceIdentityProps) {
  return (
    <div className="min-w-0">
      <div className="flex items-center gap-2">
        <h3 className={cn(
          'font-display font-semibold text-cloud-elements-textPrimary truncate',
          titleClassName ?? 'text-sm',
        )}
        >
          {name}
        </h3>
        <StatusBadge status={status as any} labelOverride={statusLabel} />
        {teeEnabled && teeStyle === 'pill' && (
          <span className="text-xs text-violet-700 dark:text-violet-400 font-data bg-violet-500/10 px-2 py-0.5 rounded-full">TEE</span>
        )}
        {teeEnabled && teeStyle === 'text' && (
          <span className="text-xs text-violet-400 font-data">TEE</span>
        )}
      </div>
      <div className="flex items-center gap-3 mt-1">
        <span className="text-xs font-data text-cloud-elements-textTertiary">{image}</span>
        <span className="text-cloud-elements-dividerColor">·</span>
        <span className="text-xs font-data text-cloud-elements-textTertiary">{specs}</span>
      </div>
    </div>
  );
}
