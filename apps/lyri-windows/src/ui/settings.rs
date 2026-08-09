use std::net::{IpAddr, SocketAddr};

use windows_reactor::*;

const DEFAULT_LRCLIB_URL: &str = "https://lrclib.net";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SongSource {
    System,
    ExternalLyri,
}

impl SongSource {
    fn selected_index(self) -> i32 {
        match self {
            Self::System => 0,
            Self::ExternalLyri => 1,
        }
    }

    fn from_selected_index(index: i32) -> Self {
        match index {
            1 => Self::ExternalLyri,
            _ => Self::System,
        }
    }
}

/// Where playback state comes from, as configured in the settings pane.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SourceConfig {
    System,
    External { host: String, port: u16 },
}

/// Web-view hosting configuration, present when hosting is enabled.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct WebServerConfig {
    bind_address: String,
    port: u16,
    pub(super) allow_seek: bool,
}

impl WebServerConfig {
    pub(super) fn socket_addr(&self) -> std::result::Result<SocketAddr, String> {
        self.bind_address
            .trim()
            .parse::<IpAddr>()
            .map(|ip| SocketAddr::new(ip, self.port))
            .map_err(|_| format!("Invalid bind address: {}", self.bind_address.trim()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SettingsState {
    song_source: SongSource,
    external_lyri_address: String,
    external_lyri_port: u16,
    lrclib_server_url: String,
    web_server_enabled: bool,
    web_server_bind_address: String,
    web_server_port: u16,
    web_server_allow_seek: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            song_source: SongSource::System,
            external_lyri_address: "127.0.0.1".to_string(),
            external_lyri_port: 8080,
            lrclib_server_url: DEFAULT_LRCLIB_URL.to_string(),
            web_server_enabled: false,
            web_server_bind_address: "0.0.0.0".to_string(),
            web_server_port: 8080,
            web_server_allow_seek: false,
        }
    }
}

impl SettingsState {
    pub(super) fn source(&self) -> SourceConfig {
        match self.song_source {
            SongSource::System => SourceConfig::System,
            SongSource::ExternalLyri => SourceConfig::External {
                host: self.external_lyri_address.trim().to_string(),
                port: self.external_lyri_port,
            },
        }
    }

    pub(super) fn lrclib_url(&self) -> String {
        let url = self.lrclib_server_url.trim();
        if url.is_empty() {
            DEFAULT_LRCLIB_URL.to_string()
        } else {
            url.to_string()
        }
    }

    pub(super) fn web_server(&self) -> Option<WebServerConfig> {
        self.web_server_enabled.then(|| WebServerConfig {
            bind_address: self.web_server_bind_address.clone(),
            port: self.web_server_port,
            allow_seek: self.web_server_allow_seek,
        })
    }
}

/// A cleared NumberBox reports NaN; callers keep the previous port then.
fn parse_port(value: f64) -> Option<u16> {
    value
        .is_finite()
        .then(|| value.round().clamp(1.0, 65535.0) as u16)
}

/// Builds a change handler that clones the current settings, applies one
/// field mutation, and publishes the result.
fn update_settings<T: 'static>(
    settings: &SettingsState,
    set_settings: &SetState<SettingsState>,
    apply: impl Fn(&mut SettingsState, T) + 'static,
) -> impl Fn(T) + 'static {
    let settings = settings.clone();
    let set_settings = set_settings.clone();
    move |value| {
        let mut next = settings.clone();
        apply(&mut next, value);
        set_settings.call(next);
    }
}

pub(super) fn render_settings_pane(
    settings: SettingsState,
    set_settings: SetState<SettingsState>,
    is_dark: bool,
    web_server_error: Option<&str>,
    close_settings: impl Fn() + 'static,
) -> Element {
    let close_button = button("\u{E711}")
        .on_click(close_settings)
        .font_family("Segoe MDL2 Assets")
        .font_size(12.0)
        .width(32.0)
        .height(30.0)
        .padding(0.0)
        .tooltip("Close settings");

    let header = grid((
        text_block("Settings")
            .font_size(20.0)
            .semibold()
            .vertical_alignment(VerticalAlignment::Center)
            .grid_column(0),
        close_button.grid_column(1),
    ))
    .columns([GridLength::Star(1.0), GridLength::Auto])
    .column_spacing(12.0)
    .horizontal_alignment(HorizontalAlignment::Stretch);

    let appearance = settings_section(
        "Appearance",
        ToggleSwitch::new(is_dark)
            .header("Theme")
            .on_content("Dark")
            .off_content("Light")
            .on_changed(|dark| {
                set_requested_theme(if dark {
                    RequestedTheme::Dark
                } else {
                    RequestedTheme::Light
                });
            }),
    );

    let source_picker = ComboBox::new(["System", "External Lyri"])
        .header("Source")
        .selected_index(settings.song_source.selected_index())
        .on_selection_changed(update_settings(
            &settings,
            &set_settings,
            |settings, index: i32| settings.song_source = SongSource::from_selected_index(index),
        ));

    let mut song_source_items = vec![Element::from(source_picker)];
    if settings.song_source == SongSource::ExternalLyri {
        let external_address = text_box(settings.external_lyri_address.clone())
            .header("IP address")
            .placeholder("127.0.0.1")
            .on_changed(update_settings(
                &settings,
                &set_settings,
                |settings, value: String| settings.external_lyri_address = value,
            ));

        let external_port = NumberBox::new(f64::from(settings.external_lyri_port))
            .header("Port")
            .range(1.0, 65535.0)
            .on_value_changed(update_settings(
                &settings,
                &set_settings,
                |settings, value: f64| {
                    if let Some(port) = parse_port(value) {
                        settings.external_lyri_port = port;
                    }
                },
            ));

        song_source_items.push(Element::from(external_address));
        song_source_items.push(Element::from(external_port));
    }

    let song_source = settings_section(
        "Song Source",
        vstack(song_source_items)
            .spacing(14.0)
            .horizontal_alignment(HorizontalAlignment::Stretch),
    );

    let lrclib = settings_section(
        "LRCLIB",
        text_box(settings.lrclib_server_url.clone())
            .header("Server URL")
            .placeholder(DEFAULT_LRCLIB_URL)
            .on_changed(update_settings(
                &settings,
                &set_settings,
                |settings, value: String| settings.lrclib_server_url = value,
            )),
    );

    let web_enabled = settings.web_server_enabled;
    let host_toggle = ToggleSwitch::new(web_enabled)
        .header("Host local web view")
        .on_content("On")
        .off_content("Off")
        .on_changed(update_settings(
            &settings,
            &set_settings,
            |settings, value: bool| settings.web_server_enabled = value,
        ));

    let bind_address = text_box(settings.web_server_bind_address.clone())
        .header("Bind address")
        .placeholder("0.0.0.0")
        .enabled(web_enabled)
        .on_changed(update_settings(
            &settings,
            &set_settings,
            |settings, value: String| settings.web_server_bind_address = value,
        ));

    let port = NumberBox::new(f64::from(settings.web_server_port))
        .header("Port")
        .range(1.0, 65535.0)
        .enabled(web_enabled)
        .on_value_changed(update_settings(
            &settings,
            &set_settings,
            |settings, value: f64| {
                if let Some(port) = parse_port(value) {
                    settings.web_server_port = port;
                }
            },
        ));

    let allow_seek = ToggleSwitch::new(settings.web_server_allow_seek)
        .header("Allow viewers to seek")
        .on_content("Allowed")
        .off_content("View only")
        .enabled(web_enabled)
        .on_changed(update_settings(
            &settings,
            &set_settings,
            |settings, value: bool| settings.web_server_allow_seek = value,
        ));

    let mut web_server_items = vec![
        Element::from(host_toggle),
        Element::from(bind_address),
        Element::from(port),
        Element::from(allow_seek),
    ];
    if let Some(error) = web_server_error {
        web_server_items.push(Element::from(
            text_block(error).font_size(12.0).opacity(0.8).wrap(),
        ));
    }

    let web_server = settings_section(
        "Web Server",
        vstack(web_server_items)
            .spacing(14.0)
            .horizontal_alignment(HorizontalAlignment::Stretch),
    );

    let content = scroll_view(
        vstack((header, appearance, song_source, lrclib, web_server))
            .spacing(22.0)
            .horizontal_alignment(HorizontalAlignment::Stretch),
    )
    .vertical_scroll_bar_visibility(ScrollViewScrollBarVisibility::Auto);

    border(content)
        .padding(22.0)
        .corner_radius(8.0)
        .background(ThemeRef::SolidBackground)
        .border_brush(ThemeRef::CardStroke)
        .border_thickness(Thickness::uniform(1.0))
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .vertical_alignment(VerticalAlignment::Stretch)
        .into()
}

fn settings_section(title: &str, content: impl Into<Element>) -> Element {
    vstack((
        text_block(title).font_size(13.0).semibold().opacity(0.68),
        border(content).padding(Thickness {
            left: 0.0,
            top: 2.0,
            right: 0.0,
            bottom: 0.0,
        }),
    ))
    .spacing(10.0)
    .horizontal_alignment(HorizontalAlignment::Stretch)
    .into()
}
