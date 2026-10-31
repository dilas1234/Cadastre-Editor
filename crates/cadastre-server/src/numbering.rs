//! Official parcel numbering (FR-NUM-01).
//!
//! Numbers follow the Aumentum UPIN convention: `PL` + a 12-digit zero-padded sequence value
//! (e.g. `PL000000015533`). The sequence itself is allocated by `cadastre.parcel_number_seq` at
//! posting time; this module only formats it.

/// Format an allocated sequence value as a UPIN.
pub fn format_upin(seq: i64) -> String {
    format!("PL{seq:012}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_pl_prefix_and_twelve_digits() {
        assert_eq!(format_upin(2), "PL000000000002");
        assert_eq!(format_upin(15_533), "PL000000015533");
        assert_eq!(format_upin(999_999_999_999), "PL999999999999");
    }
}
