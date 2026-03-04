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
        <CardDescription>Verify the Trusted Execution Environment attestation for this {subjectLabel}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Button size="sm" onClick={onFetch} disabled={busy}>
          <div className="i-ph:shield-check text-sm" />
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
