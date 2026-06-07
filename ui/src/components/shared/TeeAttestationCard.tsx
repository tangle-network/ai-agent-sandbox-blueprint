import { Card, CardContent, CardDescription, CardHeader, CardTitle, Button } from '@tangle-network/blueprint-ui/components';
import { bytesToHex, type AttestationData } from '~/lib/tee';
import { LabeledValueRow } from './LabeledValueRow';

interface TeeAttestationCardProps {
  subjectLabel: string;
  attestation: AttestationData | null;
  busy: boolean;
  error: string | null;
  onFetch: () => void;
}

export function TeeAttestationCard({
  subjectLabel,
  attestation,
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
        <div className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-3">
          <div className="i-ph:seal-warning text-base text-amber-400 mt-0.5 shrink-0" />
          <div className="space-y-0.5">
            <p className="text-xs font-medium text-amber-300">Not cryptographically verified</p>
            <p className="text-xs text-cloud-elements-textSecondary">
              This is the raw attestation the operator returned. The quote signature is not yet checked
              against a hardware root of trust and the measurement is not pinned to a known-good image, so it
              cannot prove the workload ran in a genuine enclave. Treat it as informational until verification
              ships.
            </p>
          </div>
        </div>

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
