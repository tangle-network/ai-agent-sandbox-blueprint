import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { TeeAttestationCard } from './TeeAttestationCard';
import type { AttestationData, AttestationVerification } from '~/lib/tee';

// Pins the single most security-critical UI invariant: the emerald
// "Cryptographically verified" banner renders ONLY when the server verdict is
// `verified`; every other state (unverified / measurement_mismatch / null verdict,
// including a present attestation with no verdict) must render the amber
// "Not cryptographically verified" banner and never the emerald copy.

const VERIFIED_TEXT = 'Cryptographically verified';
const UNVERIFIED_TEXT = 'Not cryptographically verified';

const attestation: AttestationData = {
  tee_type: 'Tdx',
  evidence: [1, 2, 3, 4],
  measurement: [0xab, 0xcd],
  timestamp: 1_700_000_000,
};

function verification(
  verdict: AttestationVerification['verdict'],
  overrides: Partial<AttestationVerification> = {},
): AttestationVerification {
  return {
    verdict,
    signature_verified: false,
    measurement_matched: false,
    report_data_matched: true,
    structural_ok: true,
    ...overrides,
  };
}

function renderCard(verification: AttestationVerification | null) {
  return render(
    <TeeAttestationCard
      subjectLabel="sandbox"
      attestation={attestation}
      verification={verification}
      busy={false}
      error={null}
      onFetch={() => {}}
    />,
  );
}

describe('TeeAttestationCard verification banner', () => {
  it('shows the emerald verified banner ONLY for the `verified` verdict', () => {
    renderCard(
      verification({ verdict: 'verified' }, { signature_verified: true, measurement_matched: true }),
    );
    expect(screen.getByText(VERIFIED_TEXT)).toBeInTheDocument();
    expect(screen.queryByText(UNVERIFIED_TEXT)).not.toBeInTheDocument();
  });

  it('shows the amber banner (never verified) for an `unverified` verdict', () => {
    renderCard(verification({ verdict: 'unverified', reason: 'bad signature chain' }));
    expect(screen.getByText(UNVERIFIED_TEXT)).toBeInTheDocument();
    expect(screen.queryByText(VERIFIED_TEXT)).not.toBeInTheDocument();
    // The honest reason is surfaced so the user knows why it failed.
    expect(screen.getByText(/bad signature chain/)).toBeInTheDocument();
  });

  it('shows the amber banner for a `measurement_mismatch` verdict', () => {
    renderCard(verification({ verdict: 'measurement_mismatch' }, { signature_verified: true }));
    expect(screen.getByText(UNVERIFIED_TEXT)).toBeInTheDocument();
    expect(screen.queryByText(VERIFIED_TEXT)).not.toBeInTheDocument();
  });

  it('shows the amber banner when an attestation is present but verification is null', () => {
    // A fetched report with no server verdict must NOT produce a green banner.
    renderCard(null);
    expect(screen.getByText(UNVERIFIED_TEXT)).toBeInTheDocument();
    expect(screen.queryByText(VERIFIED_TEXT)).not.toBeInTheDocument();
  });

  it('does not derive verification from raw signature/measurement booleans alone', () => {
    // Even with both booleans true, an `unverified` verdict must stay amber —
    // the verdict is the single source of truth, not the component fields.
    renderCard(
      verification(
        { verdict: 'unverified', reason: 'possible replay' },
        { signature_verified: true, measurement_matched: true },
      ),
    );
    expect(screen.getByText(UNVERIFIED_TEXT)).toBeInTheDocument();
    expect(screen.queryByText(VERIFIED_TEXT)).not.toBeInTheDocument();
  });
});
