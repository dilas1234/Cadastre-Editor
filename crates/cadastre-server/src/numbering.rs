//! Official parcel numbering (FR-NUM-01).
//!
//! Numbers follow the Aumentum UPIN convention: `PL` + a 12-digit zero-padded sequence value
//! (e.g. `PL000000015533`). The sequence itself is allocated by `cadastre.parcel_number_seq` at
//! posting time; this module only formats it.

/// Format an allocated sequence value as a UPIN.
pub fn format_upin(seq: i64) -> String {
    format!("PL{seq:012}")
}

/// How a typed parcel-number search should match.
#[derive(Debug, PartialEq, Eq)]
pub struct NumberQuery {
    /// The full UPIN when the input is a plot number (`5621`, `PL5621`, `pl 0005621`): surveyors
    /// type the short form, so it is zero-padded to `PL` + 12 digits and ranked first.
    pub exact: Option<String>,
    /// A case-insensitive `ILIKE` pattern matching the cleaned input anywhere in the number
    /// (`%` and `_` in the input are escaped, so they match literally).
    pub contains: String,
}

/// Normalise what a user typed into a parcel-number search. `None` when there is nothing to
/// search for (blank or whitespace only).
pub fn parse_number_query(input: &str) -> Option<NumberQuery> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    if cleaned.is_empty() {
        return None;
    }
    let digits = cleaned.strip_prefix("PL").unwrap_or(&cleaned);
    let exact =
        (!digits.is_empty() && digits.len() <= 12 && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse::<i64>().ok().map(format_upin))
            .flatten();
    let escaped = cleaned
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    Some(NumberQuery {
        exact,
        contains: format!("%{escaped}%"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_plot_numbers_expand_to_the_full_upin() {
        for input in [
            "5621",
            "PL5621",
            "pl5621",
            " PL 0000 0000 5621 ",
            "PL000000005621",
        ] {
            let q = parse_number_query(input).unwrap();
            assert_eq!(q.exact.as_deref(), Some("PL000000005621"), "{input}");
        }
    }

    #[test]
    fn partial_text_matches_anywhere_and_escapes_wildcards() {
        let q = parse_number_query("pl0000_5%").unwrap();
        assert_eq!(q.exact, None);
        assert_eq!(q.contains, r"%PL0000\_5\%%");
        assert_eq!(parse_number_query("562").unwrap().contains, "%562%");
    }

    #[test]
    fn blank_input_is_no_query_and_overlong_digits_are_not_a_upin() {
        assert_eq!(parse_number_query("   "), None);
        assert_eq!(parse_number_query("1234567890123").unwrap().exact, None); // 13 digits
    }

    #[test]
    fn formats_with_pl_prefix_and_twelve_digits() {
        assert_eq!(format_upin(2), "PL000000000002");
        assert_eq!(format_upin(15_533), "PL000000015533");
        assert_eq!(format_upin(999_999_999_999), "PL999999999999");
    }
}
