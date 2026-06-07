# TEE attestation test vectors

Known-good hardware-attestation samples used by the positive verification tests
in `src/tee/verify.rs`. Each was copied verbatim from the upstream crate that
verifies it, so the positive tests exercise the exact same bytes the crate
authors vouch for.

| file                         | source crate / path                                  | what it proves |
|------------------------------|------------------------------------------------------|----------------|
| `tdx_quote.bin`              | `dcap-qvl 0.5.2` `sample/tdx_quote`                   | a genuine Intel TDX DCAP quote |
| `tdx_quote_collateral.json`  | `dcap-qvl 0.5.2` `sample/tdx_quote_collateral.json`  | matching Intel PCS collateral (TCB info, CRLs, QE identity) for the quote above |
| `sev_vcek_milan.der`         | `sev 8.0.0` `tests/certs_data/vcek_milan.der`        | a genuine AMD Milan VCEK leaf certificate |
| `sev_report_milan.hex`       | `sev 8.0.0` `tests/certs_data/report_milan.hex`      | a genuine AMD SEV-SNP attestation report signed by the VCEK above |

The AMD ARK/ASK roots that anchor the SEV chain are the ones built into the
`sev` crate (`sev::certs::snp::builtin::milan`); the Intel SGX Root CA that
anchors the TDX chain is the one built into `dcap-qvl` (`QuoteVerifier::new_prod`).
Neither root is checked into this repo — pinning lives in the audited crates.

No genuine AWS Nitro attestation document vector is available from any vendored
crate, so the Nitro arm is intentionally honest-`Err` (see `src/tee/verify.rs`).
