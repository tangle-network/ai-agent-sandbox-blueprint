pub mod batch;
pub mod exec;
pub mod sandbox;
pub mod ssh;
pub mod workflow;

/// Convert a raw 20-byte EVM caller address to a lowercase hex string with `0x` prefix.
pub(crate) fn caller_hex(bytes: &[u8; 20]) -> String {
    let mut s = String::with_capacity(42);
    s.push_str("0x");
    for b in bytes {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
    }
    s
}
