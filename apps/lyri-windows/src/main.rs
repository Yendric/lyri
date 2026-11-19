#![windows_subsystem = "windows"]

mod external_source;
mod now_playing;
mod ui;

use windows_reactor::{App, WindowPolicy};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    App::run_component_with_policy::<ui::Lyri>((), WindowPolicy::new().client_size(520.0, 750.0))?;
    Ok(())
}
