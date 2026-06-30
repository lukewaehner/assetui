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
