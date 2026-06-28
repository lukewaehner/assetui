#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    ById,
    ByName,
    ByPrice,
    ByPrevClose,
    ByVolume,
    ByAsOf,
}
