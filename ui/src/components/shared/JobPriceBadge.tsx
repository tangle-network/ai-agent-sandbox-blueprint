import { useStore } from '@nanostores/react';
import { infraStore } from '~/lib/stores/infra';
import { useJobPrice } from '~/lib/hooks/useJobPrice';
import { formatCost } from '~/lib/hooks/useQuotes';
import { cn } from '~/lib/utils';

interface JobPriceBadgeProps {
  jobIndex: number;
  pricingMultiplier: number;
  className?: string;
  /** Show loading state inline */
  compact?: boolean;
}

/**
 * Inline badge that displays the RFQ-resolved price for a job.
 * Falls back to multiplier-based estimate if no operator RPC is available.
 *
 * Usage:
 *   <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_CREATE} pricingMultiplier={50} />
 */
export function JobPriceBadge({ jobIndex, pricingMultiplier, className, compact }: JobPriceBadgeProps) {
  const infra = useStore(infraStore);
  const operatorRpcUrl = infra.serviceInfo?.operators?.[0]?.rpcAddress;
  const serviceId = BigInt(infra.serviceId || '0');
  const blueprintId = BigInt(infra.blueprintId || '0');

  const { quote, isLoading, isSolvingPow, formattedPrice, error } = useJobPrice(
    operatorRpcUrl,
    serviceId,
    jobIndex,
    blueprintId,
    !!operatorRpcUrl && serviceId > 0n,
  );

  // Fallback: estimate from multiplier (base rate = 0.001 TNT = 1e15 wei)
  const estimatedPrice = BigInt(pricingMultiplier) * 1_000_000_000_000_000n;
  const estimatedFormatted = formatCost(estimatedPrice);

  const hasRfqPrice = quote && !error;
  const displayPrice = hasRfqPrice ? formattedPrice : `~${estimatedFormatted}`;
  const isEstimate = !hasRfqPrice;

  if (compact) {
    return (
      <span className={cn('text-xs font-data', className)}>
        {isLoading || isSolvingPow ? (
          <span className="text-cloud-elements-textTertiary animate-pulse">...</span>
        ) : (
          <span className={isEstimate ? 'text-cloud-elements-textTertiary' : 'text-teal-400'}>
            {displayPrice}
          </span>
        )}
      </span>
    );
  }

  return (
    <div
      className={cn(
        'inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-xs font-data',
        isEstimate
          ? 'bg-cloud-elements-background-depth-2 text-cloud-elements-textSecondary'
          : 'bg-teal-500/10 text-teal-400 border border-teal-500/20',
        className,
      )}
    >
      {isLoading || isSolvingPow ? (
        <>
          <div className="i-ph:circle-fill text-[8px] animate-pulse" />
          <span>Quoting...</span>
        </>
      ) : (
        <>
          <div className={cn('text-[10px]', isEstimate ? 'i-ph:approximately-equals' : 'i-ph:tag')} />
          <span>{displayPrice}</span>
        </>
      )}
    </div>
  );
}
