#![windows_subsystem = "windows"]

mod now_playing;
mod ui;

use windows_reactor::*;

fn main() {
    let _ = App::new()
        .title("Lyri")
        .inner_size(520.0, 750.0)
        .backdrop(Backdrop::Mica)
        .render(ui::app);
}
