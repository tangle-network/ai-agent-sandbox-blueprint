import { TeeAttestationCard } from '~/components/shared/TeeAttestationCard';
import type { AttestationData, AttestationVerification } from '~/lib/tee';

interface AttestationTabProps {
  attestation: AttestationData | null;
  attestationVerification: AttestationVerification | null;
  attestationBusy: boolean;
  attestationError: string | null;
  handleFetchAttestation: () => Promise<void>;
}

export function AttestationTab({
  attestation,
  attestationVerification,
  attestationBusy,
  attestationError,
  handleFetchAttestation,
}: AttestationTabProps) {
  return (
    <div className="space-y-4">
      <TeeAttestationCard
        subjectLabel="sandbox"
        attestation={attestation}
        verification={attestationVerification}
        busy={attestationBusy}
        error={attestationError}
        onFetch={handleFetchAttestation}
      />
    </div>
  );
}
