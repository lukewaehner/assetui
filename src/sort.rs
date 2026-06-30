// Sort configuration shared between the TUI (which selects it) and the
// fetch layer (which translates it into SQL ordering).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

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
