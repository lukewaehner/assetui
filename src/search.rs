//! Case-insensitive subsequence matching used by the TUI's `/` fuzzy filter
//! and its per-character match highlighting.
//!
//! Adapted from the tuxedo project's search module: one matcher returns the
//! matched byte offsets so filtering and highlighting can never disagree.

/// Returns byte offsets in `haystack` where each char of `needle` is matched
/// in order, case-insensitively, with arbitrary gaps allowed. Returns `None`
/// when not every needle char can be matched, or when `needle` is empty.
///
/// Offsets are into the original `haystack` (not a lowercased copy), so they
/// land on `char_indices` boundaries and are safe to slice.
pub fn subseq_match_ci(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    if needle.is_empty() {
        return None;
    }
    let needle_lower: Vec<String> = needle
        .chars()
        .map(|c| c.to_lowercase().collect::<String>())
        .collect();
    let mut positions = Vec::with_capacity(needle_lower.len());
    let mut idx = 0;
    for (byte, ch) in haystack.char_indices() {
        if idx == needle_lower.len() {
            break;
        }
        let ch_lower: String = ch.to_lowercase().collect();
        if ch_lower == needle_lower[idx] {
            positions.push(byte);
            idx += 1;
        }
    }
    (idx == needle_lower.len()).then_some(positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_contiguous_substring() {
        // Greedy: each needle char takes the earliest available haystack char,
        // so "AP" lands on the first A.
        assert_eq!(subseq_match_ci("AAPL", "AP"), Some(vec![0, 2]));
    }

    #[test]
    fn test_matches_subsequence_with_gaps() {
        // "apn" finds A, p, n in "Apple Inc." with gaps.
        assert_eq!(subseq_match_ci("Apple Inc.", "apn"), Some(vec![0, 1, 7]));
    }

    #[test]
    fn test_case_insensitive_both_directions() {
        assert_eq!(subseq_match_ci("AAPL", "aap"), Some(vec![0, 1, 2]));
        assert_eq!(subseq_match_ci("nvda", "NVD"), Some(vec![0, 1, 2]));
    }

    #[test]
    fn test_empty_needle_is_none() {
        assert_eq!(subseq_match_ci("anything", ""), None);
    }

    #[test]
    fn test_missing_chars_return_none() {
        assert_eq!(subseq_match_ci("AAPL", "xyz"), None);
    }

    #[test]
    fn test_order_matters() {
        // Subsequence is in-order: "LP" can't match "AAPL".
        assert_eq!(subseq_match_ci("AAPL", "LP"), None);
    }

    #[test]
    fn test_offsets_land_on_char_boundaries_for_unicode() {
        // "Café" byte layout: C(0) a(1) f(2) é(3..5).
        assert_eq!(subseq_match_ci("Café SA", "cé"), Some(vec![0, 3]));
    }
}
