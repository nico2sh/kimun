use crossterm::event::{KeyEvent, MouseEvent};

pub enum Event {
    Quit,
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
}
