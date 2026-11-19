use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

use windows_reactor::*;

use super::{Lyri, Message};

const DEFAULT_LRCLIB_URL: &str = "https://lrclib.net";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SongSource {
    System,
    ExternalLyri,
}

impl SongSource {
    fn selected_index(self) -> usize {
        match self {
            Self::System => 0,
            Self::ExternalLyri => 1,
        }
    }

    fn from_selected_index(index: usize) -> Self {
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

/// One edit made in the settings pane.
#[derive(Clone, Debug)]
pub enum SettingsChange {
    SongSource(Option<usize>),
    ExternalAddress(String),
    ExternalPort(Option<f64>),
    LrclibUrl(String),
    WebServerEnabled(bool),
    WebServerBindAddress(String),
    WebServerPort(Option<f64>),
    WebServerAllowSeek(bool),
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

    pub(super) fn apply(&mut self, change: SettingsChange) {
        match change {
            SettingsChange::SongSource(Some(index)) => {
                self.song_source = SongSource::from_selected_index(index);
            }
            SettingsChange::SongSource(None) => {}
            SettingsChange::ExternalAddress(value) => self.external_lyri_address = value,
            SettingsChange::ExternalPort(value) => {
                if let Some(port) = parse_port(value) {
                    self.external_lyri_port = port;
                }
            }
            SettingsChange::LrclibUrl(value) => self.lrclib_server_url = value,
            SettingsChange::WebServerEnabled(value) => self.web_server_enabled = value,
            SettingsChange::WebServerBindAddress(value) => self.web_server_bind_address = value,
            SettingsChange::WebServerPort(value) => {
                if let Some(port) = parse_port(value) {
                    self.web_server_port = port;
                }
            }
            SettingsChange::WebServerAllowSeek(value) => self.web_server_allow_seek = value,
        }
    }
}

/// A cleared NumberBox reports no value; callers keep the previous port then.
fn parse_port(value: Option<f64>) -> Option<u16> {
    value
        .filter(|value| value.is_finite())
        .map(|value| value.round().clamp(1.0, 65535.0) as u16)
}

fn change<T: 'static>(context: &ViewContext<Lyri>, map: fn(T) -> SettingsChange) -> Callback<T> {
    context.callback(move |value| Message::Settings(map(value)))
}

fn text_change(
    context: &ViewContext<Lyri>,
    map: fn(String) -> SettingsChange,
) -> Callback<Rc<str>> {
    context.callback(move |value: Rc<str>| Message::Settings(map(value.to_string())))
}

pub(super) fn render_settings_pane(
    settings: &SettingsState,
    is_dark: bool,
    web_server_error: Option<&str>,
    context: &ViewContext<Lyri>,
) -> Border {
    let close_button = Button::new()
        .content(FontIcon::new().glyph("\u{E711}").font_size(12.0))
        .width(32.0)
        .height(30.0)
        .on_click(context.message(Message::CloseSettings))
        .tooltip("Close settings");

    let header = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .column_spacing(12.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .children((
            TextBlock::new()
                .text("Settings")
                .font_size(20.0)
                .font_weight(FontWeight::SEMI_BOLD)
                .vertical_alignment(VerticalAlignment::Center)
                .grid_column(0),
            Border::new().grid_column(1).content(close_button),
        ));

    let appearance = settings_section(
        "Appearance",
        ToggleSwitch::new()
            .header("Theme")
            .on_content("Dark")
            .off_content("Light")
            .is_on(is_dark)
            .on_toggled(context.callback(|dark: bool| {
                Message::SetTheme(if dark {
                    WindowTheme::Dark
                } else {
                    WindowTheme::Light
                })
            })),
    );

    let source_picker = ComboBox::new()
        .header("Source")
        .items_source(["System", "External Lyri"])
        .selected_index(settings.song_source.selected_index())
        .on_selection_changed(change(context, SettingsChange::SongSource));

    let mut song_source_items: Vec<View> = vec![source_picker.into()];
    if settings.song_source == SongSource::ExternalLyri {
        song_source_items.push(
            TextBox::new()
                .header("IP address")
                .placeholder_text("127.0.0.1")
                .text(&settings.external_lyri_address)
                .on_text_changed(text_change(context, SettingsChange::ExternalAddress))
                .into(),
        );
        song_source_items.push(
            port_box(settings.external_lyri_port, true)
                .on_value_changed(change(context, SettingsChange::ExternalPort))
                .into(),
        );
    }

    let song_source = settings_section(
        "Song Source",
        StackPanel::new()
            .spacing(14.0)
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .children(song_source_items),
    );

    let lrclib = settings_section(
        "LRCLIB",
        TextBox::new()
            .header("Server URL")
            .placeholder_text(DEFAULT_LRCLIB_URL)
            .text(&settings.lrclib_server_url)
            .on_text_changed(text_change(context, SettingsChange::LrclibUrl)),
    );

    let web_enabled = settings.web_server_enabled;
    let mut web_server_items: Vec<View> = vec![
        ToggleSwitch::new()
            .header("Host local web view")
            .on_content("On")
            .off_content("Off")
            .is_on(web_enabled)
            .on_toggled(change(context, SettingsChange::WebServerEnabled))
            .into(),
        TextBox::new()
            .header("Bind address")
            .placeholder_text("0.0.0.0")
            .text(&settings.web_server_bind_address)
            .is_enabled(web_enabled)
            .on_text_changed(text_change(context, SettingsChange::WebServerBindAddress))
            .into(),
        port_box(settings.web_server_port, web_enabled)
            .on_value_changed(change(context, SettingsChange::WebServerPort))
            .into(),
        ToggleSwitch::new()
            .header("Allow viewers to seek")
            .on_content("Allowed")
            .off_content("View only")
            .is_on(settings.web_server_allow_seek)
            .is_enabled(web_enabled)
            .on_toggled(change(context, SettingsChange::WebServerAllowSeek))
            .into(),
    ];
    if let Some(error) = web_server_error {
        web_server_items.push(
            TextBlock::new()
                .text(error)
                .font_size(12.0)
                .opacity(0.8)
                .text_wrapping(TextWrapping::Wrap)
                .into(),
        );
    }

    let web_server = settings_section(
        "Web Server",
        StackPanel::new()
            .spacing(14.0)
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .children(web_server_items),
    );

    let content = ScrollView::new()
        .vertical_scroll_bar_visibility(ScrollingScrollBarVisibility::Auto)
        .content(
            StackPanel::new()
                .spacing(22.0)
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .children((header, appearance, song_source, lrclib, web_server)),
        );

    Border::new()
        .padding(22.0)
        .corner_radius(8.0)
        .background(ThemeBrush::SolidBackground)
        .border_brush(ThemeBrush::CardStroke)
        .border_thickness(1.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .vertical_alignment(VerticalAlignment::Stretch)
        .content(content)
}

fn port_box(port: u16, enabled: bool) -> NumberBox {
    NumberBox::new()
        .header("Port")
        .minimum(1.0)
        .maximum(65535.0)
        .value(f64::from(port))
        .is_enabled(enabled)
}

fn settings_section(title: &str, content: impl Into<View>) -> View {
    StackPanel::new()
        .spacing(10.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .children((
            TextBlock::new()
                .text(title)
                .font_size(13.0)
                .font_weight(FontWeight::SEMI_BOLD)
                .opacity(0.68),
            Border::new()
                .padding(Thickness::new(0.0, 2.0, 0.0, 0.0))
                .content(content),
        ))
        .into()
}
