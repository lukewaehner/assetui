use crate::tui::app::Page;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Tick,
    Render,
    Resize(u16, u16),
    Quit,
    ToggleHelp,
    /// Switch to `Page`, running its lazy page-entry effects.
    GoToPage(Page),
    /// Dismiss whatever overlay is currently showing.
    CloseOverlay,
}
