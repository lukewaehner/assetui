/// Direction of a sort operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Column to sort by when querying the quotes table.
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
