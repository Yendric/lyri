#[cfg(target_os = "linux")]
mod app;
#[cfg(target_os = "linux")]
mod external_source;
#[cfg(target_os = "linux")]
mod now_playing;

#[cfg(target_os = "linux")]
fn main() -> gtk::glib::ExitCode {
    app::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("lyri-gtk is only available on Linux.");
}
