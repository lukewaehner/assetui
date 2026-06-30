//! Sort configuration shared between the TUI (which selects it via keystrokes)
//! and the fetch layer (which translates it into SQL `ORDER BY` clauses).

/// Direction of a sort operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Column to sort by when querying the quotes table.
///
/// Each variant maps 1:1 to a column name in `fetch_sorted`.  The mapping
/// lives there rather than here so the database layer owns all SQL concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    ById,
    ByTicker,
    ByName,
    ByPrice,
    ByPrevClose,
    ByVolume,
    ByAsOf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_order_clone() {
        let original = SortOrder::Ascending;
        let cloned = original.clone();
        assert_eq!(cloned, SortOrder::Ascending);
    }

    #[test]
    fn test_sort_order_copy() {
        let a = SortOrder::Descending;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn test_sort_mode_all_variants_are_distinct() {
        let mut variants = vec![
            SortMode::ById,
            SortMode::ByTicker,
            SortMode::ByName,
            SortMode::ByPrice,
            SortMode::ByPrevClose,
            SortMode::ByVolume,
            SortMode::ByAsOf,
        ];
        variants.dedup();
        assert_eq!(variants.len(), 7);
    }

    #[test]
    fn test_sort_mode_debug() {
        let debug_str = format!("{:?}", SortMode::ByPrice);
        assert_eq!(debug_str, "ByPrice");
    }
}
