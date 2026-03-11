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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_hex_all_zeros() {
        let result = caller_hex(&[0u8; 20]);
        assert_eq!(result, "0x0000000000000000000000000000000000000000");
    }

    #[test]
    fn caller_hex_all_ff() {
        let result = caller_hex(&[0xff; 20]);
        assert_eq!(result, "0xffffffffffffffffffffffffffffffffffffffff");
    }

    #[test]
    fn caller_hex_known_address() {
        let bytes: [u8; 20] = [
            0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
            0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        ];
        assert_eq!(
            caller_hex(&bytes),
            "0xdeadbeef00112233445566778899aabbccddeeff"
        );
    }

    #[test]
    fn caller_hex_length_always_42() {
        for byte in [0u8, 0x7f, 0xff] {
            let result = caller_hex(&[byte; 20]);
            assert_eq!(result.len(), 42);
            assert!(result.starts_with("0x"));
        }
    }
}
