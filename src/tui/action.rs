#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Tick,
    Render,
    Resize(u16, u16),
    Quit,
    ToggleHelp,
}
