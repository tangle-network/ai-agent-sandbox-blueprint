import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';
import { useOnChainVerification } from '~/lib/hooks/useInstanceReads';
import { CopyButton } from './CopyButton';
import { truncateAddress } from '@tangle-network/agent-ui/primitives';

const BYTES32_ZERO = '0x0000000000000000000000000000000000000000000000000000000000000000';

type VerificationStatus = 'match' | 'mismatch' | 'warning' | 'loading' | 'error' | 'na';

export interface VerificationRow {
  label: string;
  status: VerificationStatus;
  value: string;
  detail?: string;
  copyValue?: string;
}

interface OnChainVerificationCardProps {
  serviceId: bigint | null;
  operator: string | undefined;
  sidecarUrl: string | undefined;
  blueprintType: 'instance' | 'tee-instance';
  className?: string;
}

export function computeVerificationRows({
  isProvisioned,
  isOperatorProvisioned,
  operatorCount,
  operatorEndpoints,
  attestationHash,
  apiOperator,
  apiSidecarUrl,
  isTee,
}: {
  isProvisioned: { data?: boolean; isLoading: boolean; error: Error | null };
  isOperatorProvisioned: { data?: boolean; isLoading: boolean; error: Error | null };
  operatorCount: { data?: number; isLoading: boolean; error: Error | null };
  operatorEndpoints: {
    data?: readonly [readonly `0x${string}`[], readonly string[]];
    isLoading: boolean;
    error: Error | null;
  };
  attestationHash: { data?: `0x${string}`; isLoading: boolean; error: Error | null };
  apiOperator: string | undefined;
  apiSidecarUrl: string | undefined;
  isTee: boolean;
}): VerificationRow[] {
  const rows: VerificationRow[] = [];

  // Provisioned
  if (isProvisioned.isLoading) {
    rows.push({ label: 'Provisioned', status: 'loading', value: 'Loading...' });
  } else if (isProvisioned.error) {
    rows.push({ label: 'Provisioned', status: 'error', value: 'Query failed' });
  } else {
    rows.push({
      label: 'Provisioned',
      status: isProvisioned.data ? 'match' : 'warning',
      value: isProvisioned.data ? 'Yes' : 'Not provisioned on-chain',
    });
  }

  // Operator verified
  if (!apiOperator) {
    rows.push({ label: 'Operator Verified', status: 'na', value: 'No operator assigned' });
  } else if (isOperatorProvisioned.isLoading) {
    rows.push({ label: 'Operator Verified', status: 'loading', value: 'Loading...' });
  } else if (isOperatorProvisioned.error) {
    rows.push({ label: 'Operator Verified', status: 'error', value: 'Query failed' });
  } else {
    rows.push({
      label: 'Operator Verified',
      status: isOperatorProvisioned.data ? 'match' : 'mismatch',
      value: isOperatorProvisioned.data ? 'Confirmed on-chain' : 'Not confirmed on-chain',
    });
  }

  // Operator count
  if (operatorCount.isLoading) {
    rows.push({ label: 'Operator Count', status: 'loading', value: 'Loading...' });
  } else if (operatorCount.error) {
    rows.push({ label: 'Operator Count', status: 'error', value: 'Query failed' });
  } else {
    rows.push({
      label: 'Operator Count',
      status: 'match',
      value: String(operatorCount.data ?? 0),
    });
  }

  // Endpoint match
  if (!apiOperator) {
    rows.push({ label: 'Endpoint Match', status: 'na', value: 'No operator assigned' });
  } else if (operatorEndpoints.isLoading) {
    rows.push({ label: 'Endpoint Match', status: 'loading', value: 'Loading...' });
  } else if (operatorEndpoints.error) {
    rows.push({ label: 'Endpoint Match', status: 'error', value: 'Query failed' });
  } else if (operatorEndpoints.data) {
    const [operators, urls] = operatorEndpoints.data;
    const idx = operators.findIndex(
      (op) => op.toLowerCase() === apiOperator.toLowerCase(),
    );
    if (idx === -1) {
      rows.push({
        label: 'Endpoint Match',
        status: 'mismatch',
        value: 'Operator not in on-chain list',
      });
    } else {
      const onChainUrl = urls[idx];
      const matches = onChainUrl === apiSidecarUrl;
      rows.push({
        label: 'Endpoint Match',
        status: matches ? 'match' : 'mismatch',
        value: matches ? 'Verified' : 'URL mismatch',
        detail: matches
          ? undefined
          : `On-chain: ${onChainUrl || '(empty)'} | API: ${apiSidecarUrl || '(empty)'}`,
      });
    }
  }

  // Attestation hash (TEE only)
  if (isTee && apiOperator) {
    if (attestationHash.isLoading) {
      rows.push({ label: 'Attestation Hash', status: 'loading', value: 'Loading...' });
    } else if (attestationHash.error) {
      rows.push({ label: 'Attestation Hash', status: 'error', value: 'Query failed' });
    } else if (attestationHash.data) {
      const isZero = attestationHash.data === BYTES32_ZERO;
      rows.push({
        label: 'Attestation Hash',
        status: isZero ? 'warning' : 'match',
        value: isZero ? 'None' : truncateAddress(attestationHash.data),
        detail: isZero ? 'No attestation recorded for this operator' : undefined,
        copyValue: isZero ? undefined : attestationHash.data,
      });
    }
  }

  return rows;
}

const statusConfig: Record<VerificationStatus, { icon: string; color: string }> = {
  match: { icon: 'i-ph:check-circle-fill', color: 'text-teal-400' },
  mismatch: { icon: 'i-ph:warning-circle-fill', color: 'text-amber-400' },
  warning: { icon: 'i-ph:warning-fill', color: 'text-amber-400' },
  loading: { icon: 'i-ph:spinner-gap', color: 'text-cloud-elements-textTertiary animate-spin' },
  error: { icon: 'i-ph:x-circle-fill', color: 'text-red-400' },
  na: { icon: 'i-ph:minus-circle', color: 'text-cloud-elements-textTertiary' },
};

function VerificationRowDisplay({ row }: { row: VerificationRow }) {
  const { icon, color } = statusConfig[row.status];

  return (
    <div className="space-y-0.5">
      <div className="flex justify-between text-sm gap-2">
        <span className="text-cloud-elements-textSecondary shrink-0">{row.label}</span>
        <div className="flex items-center gap-1.5 min-w-0">
          <div className={cn('text-sm shrink-0', icon, color)} />
          <span className="text-cloud-elements-textPrimary truncate text-xs">{row.value}</span>
          {row.copyValue && <CopyButton value={row.copyValue} />}
        </div>
      </div>
      {row.detail && (
        <p className="text-xs text-cloud-elements-textTertiary text-right">{row.detail}</p>
      )}
    </div>
  );
}

export function OnChainVerificationCard({
  serviceId,
  operator,
  sidecarUrl,
  blueprintType,
  className,
}: OnChainVerificationCardProps) {
  const verification = useOnChainVerification({
    serviceId,
    operator,
    blueprintType,
    enabled: serviceId !== null,
  });

  const rows = computeVerificationRows({
    isProvisioned: verification.isProvisioned,
    isOperatorProvisioned: verification.isOperatorProvisioned,
    operatorCount: verification.operatorCount,
    operatorEndpoints: verification.operatorEndpoints,
    attestationHash: verification.attestationHash,
    apiOperator: operator,
    apiSidecarUrl: sidecarUrl,
    isTee: blueprintType === 'tee-instance',
  });

  return (
    <Card className={className}>
      <CardHeader>
        <CardTitle>On-Chain Verification</CardTitle>
        <CardDescription>
          Direct blockchain reads to verify operator-reported state
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {rows.map((row) => (
          <VerificationRowDisplay key={row.label} row={row} />
        ))}
      </CardContent>
    </Card>
  );
}
