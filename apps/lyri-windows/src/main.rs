#![windows_subsystem = "windows"]

mod external_source;
mod now_playing;
mod ui;

use windows_reactor::*;

fn main() -> Result<()> {
    bootstrap()?;
    App::new()
        .title("Lyri")
        .inner_size(520.0, 750.0)
        .backdrop(Backdrop::Mica)
        .render(ui::app)
}
