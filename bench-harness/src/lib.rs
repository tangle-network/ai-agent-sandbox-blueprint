//! Bench harness — statistical aggregation, run manifest, and regression detection.
//!
//! Consumes the machine-readable output that Criterion writes to
//! `target/criterion/**/new/estimates.json` and produces:
//!
//! - A per-run JSONL manifest with reproducibility metadata (git SHA, host,
//!   rustc, target triple, UTC timestamp) and the full statistical summary
//!   for every benchmark.
//! - A cross-run comparator that flags regressions using confidence-interval
//!   comparison, not point estimates.
//! - A rendered markdown report suitable for CI summary / PR comments.

pub mod compare;
pub mod criterion_ingest;
pub mod manifest;
pub mod stats;
