//! The window, drawn panel by panel. A view only reads the [`AppState`]; what the user did comes
//! back as [`UiAction`]s, which the window applies at the end of the frame.

mod bottom_bar;
mod input_pane;
mod preview_pane;
mod shortcuts;
mod target_pane;
mod top_bar;

pub use bottom_bar::bottom_bar;
pub use input_pane::input_pane;
pub use shortcuts::shortcuts;
pub use target_pane::target_pane;
pub use top_bar::top_bar;
