import { Card, CardContent, CardDescription, CardHeader, CardTitle, Button } from '@tangle-network/blueprint-ui/components';
import {
  bytesToHex,
  isAttestationVerified,
  type AttestationData,
  type AttestationVerification,
} from '~/lib/tee';
import { LabeledValueRow } from './LabeledValueRow';

interface TeeAttestationCardProps {
  subjectLabel: string;
  attestation: AttestationData | null;
  verification: AttestationVerification | null;
  busy: boolean;
  error: string | null;
  onFetch: () => void;
}

/** Honest, server-evaluated trust banner. Never claims verification when the
 *  server returned anything other than the `verified` verdict. */
function VerificationBanner({ verification }: { verification: AttestationVerification | null }) {
  if (verification && isAttestationVerified(verification)) {
    return (
      <div className="flex items-start gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 p-3">
        <div className="i-ph:seal-check text-base text-emerald-400 mt-0.5 shrink-0" />
        <div className="space-y-0.5">
          <p className="text-xs font-medium text-emerald-300">Cryptographically verified</p>
          <p className="text-xs text-cloud-elements-textSecondary">
            The quote signature chained to a hardware root of trust and the measurement matched a pinned
            known-good image. This attestation proves the workload ran in a genuine enclave.
          </p>
        </div>
      </div>
    );
  }

  const reason =
    verification?.verdict.verdict === 'unverified'
      ? verification.verdict.reason
      : verification?.verdict.verdict === 'measurement_mismatch'
        ? 'The signed measurement matched none of the pinned known-good images.'
        : 'The server has not returned a verification verdict for this report.';

  return (
    <div className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-3">
      <div className="i-ph:seal-warning text-base text-amber-400 mt-0.5 shrink-0" />
      <div className="space-y-0.5">
        <p className="text-xs font-medium text-amber-300">Not cryptographically verified</p>
        <p className="text-xs text-cloud-elements-textSecondary">
          The server could not verify this attestation against a hardware root of trust. Do not encrypt
          secrets to this enclave. Reason: {reason}
        </p>
      </div>
    </div>
  );
}

export function TeeAttestationCard({
  subjectLabel,
  attestation,
  verification,
  busy,
  error,
  onFetch,
}: TeeAttestationCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">TEE Attestation</CardTitle>
        <CardDescription>Attestation evidence reported by the operator for this {subjectLabel}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {attestation && <VerificationBanner verification={verification} />}

        <Button size="sm" onClick={onFetch} disabled={busy}>
          <div className="i-ph:shield text-sm" />
          {busy ? 'Fetching...' : attestation ? 'Refresh Attestation' : 'Get Attestation'}
        </Button>

        {error && (
          <p className="text-xs text-red-400">{error}</p>
        )}

        {attestation && (
          <div className="space-y-3">
            <LabeledValueRow label="TEE Type" value={attestation.tee_type} />
            <LabeledValueRow
              label="Timestamp"
              value={new Date(attestation.timestamp * 1000).toLocaleString()}
            />
            {verification && (
              <>
                <LabeledValueRow
                  label="Verdict"
                  value={verification.verdict.verdict.replace(/_/g, ' ')}
                />
                <LabeledValueRow
                  label="Signature verified"
                  value={verification.signature_verified ? 'yes' : 'no'}
                />
                <LabeledValueRow
                  label="Measurement pinned"
                  value={verification.measurement_matched ? 'yes' : 'no'}
                />
              </>
            )}
            <div className="space-y-1.5">
              <span className="text-sm text-cloud-elements-textSecondary">Measurement</span>
              <div className="p-3 rounded-lg bg-cloud-elements-background-depth-2">
                <code className="text-xs font-data text-cloud-elements-textPrimary break-all">
                  {bytesToHex(attestation.measurement)}
                </code>
              </div>
            </div>
            <details className="group">
              <summary className="text-sm text-cloud-elements-textSecondary cursor-pointer hover:text-cloud-elements-textPrimary transition-colors">
                Evidence ({attestation.evidence.length} bytes)
              </summary>
              <div className="mt-2 p-3 rounded-lg bg-cloud-elements-background-depth-2 max-h-48 overflow-y-auto">
                <code className="text-xs font-data text-cloud-elements-textTertiary break-all">
                  {bytesToHex(attestation.evidence)}
                </code>
              </div>
            </details>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
