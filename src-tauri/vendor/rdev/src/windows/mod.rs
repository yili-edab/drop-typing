extern crate winapi;

mod common;
mod display;
#[cfg(feature = "unstable_grab")]
mod grab;
mod keyboard;
mod keycodes;
mod listen;
mod simulate;

pub use crate::windows::display::display_size;
#[cfg(feature = "unstable_grab")]
pub use crate::windows::grab::grab;
pub use crate::windows::keyboard::Keyboard;
pub use crate::windows::listen::{listen, set_swallow_win_down, set_swallow_win_up};
pub use crate::windows::simulate::simulate;
