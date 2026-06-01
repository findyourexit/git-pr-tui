use crossterm::event::KeyEvent;

use super::effect::DataEvent;

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Resize { width: u16, height: u16 },
    Tick,
    Data(DataEvent),
}
