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
        assert_eq!(subseq_match_ci("AAPL", "AP"), Some(vec![0, 2]));
    }

    #[test]
    fn test_matches_subsequence_with_gaps() {
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
        assert_eq!(subseq_match_ci("AAPL", "LP"), None);
    }

    #[test]
    fn test_offsets_land_on_char_boundaries_for_unicode() {
        assert_eq!(subseq_match_ci("Café SA", "cé"), Some(vec![0, 3]));
    }
}
