use super::action::Action;
use crossterm::event::{Event, KeyEvent, MouseEvent};
use std::io;

#[allow(dead_code)]
pub trait Component {
    /// Initialize the component.
    fn init(&mut self) -> Result<(), io::Error> {
        Ok(())
    }
    fn handle_key_events(&mut self, _key: KeyEvent) -> Option<Action> {
        None
    }
    fn handle_mouse_events(&mut self, _mouse: MouseEvent) -> Option<Action> {
        None
    }
    fn handle_events(&mut self, event: Option<Event>) -> Option<Action> {
        match event {
            Some(Event::Key(key)) => self.handle_key_events(key),
            Some(Event::Mouse(mouse)) => self.handle_mouse_events(mouse),
            _ => None,
        }
    }
    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }
}

pub mod earnings;
pub mod help;
pub mod page_picker;
pub mod portfolio;
