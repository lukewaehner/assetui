/// Direction of a sort operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

impl SortOrder {
    /// The SQL keyword this direction maps to in an `ORDER BY` clause.
    pub fn keyword(&self) -> &'static str {
        match self {
            SortOrder::Ascending => "ASC",
            SortOrder::Descending => "DESC",
        }
    }
}

/// Column to sort by when querying the quotes table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SortMode {
    #[default]
    ById,
    ByTicker,
    ByName,
    ByPrice,
    ByPrevClose,
    ByVolume,
    ByAsOf,
}

impl SortMode {
    /// The `quotes` table column this mode sorts on.
    pub fn column(&self) -> &'static str {
        match self {
            SortMode::ById => "id",
            SortMode::ByTicker => "ticker",
            SortMode::ByName => "name",
            SortMode::ByPrice => "price",
            SortMode::ByPrevClose => "previous_close",
            SortMode::ByVolume => "day_volume",
            SortMode::ByAsOf => "as_of",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_order_clone() {
        let original = SortOrder::Ascending;
        let cloned = original;
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
        use std::collections::HashSet;
        let variants = [
            SortMode::ById,
            SortMode::ByTicker,
            SortMode::ByName,
            SortMode::ByPrice,
            SortMode::ByPrevClose,
            SortMode::ByVolume,
            SortMode::ByAsOf,
        ];
        let set: HashSet<SortMode> = variants.iter().copied().collect();
        assert_eq!(set.len(), variants.len());
    }

    #[test]
    fn test_sort_mode_debug() {
        let debug_str = format!("{:?}", SortMode::ByPrice);
        assert_eq!(debug_str, "ByPrice");
    }

    #[test]
    fn sort_mode_by_price_maps_to_price_column() {
        assert_eq!(SortMode::ByPrice.column(), "price");
    }

    #[test]
    fn sort_mode_by_prev_close_maps_to_previous_close_column() {
        assert_eq!(SortMode::ByPrevClose.column(), "previous_close");
    }

    #[test]
    fn sort_order_ascending_maps_to_asc_keyword() {
        assert_eq!(SortOrder::Ascending.keyword(), "ASC");
    }

    #[test]
    fn sort_order_descending_maps_to_desc_keyword() {
        assert_eq!(SortOrder::Descending.keyword(), "DESC");
    }
}
