use std::net::{IpAddr, SocketAddr};

use gtk::prelude::*;
use gtk::{Align, Orientation};
use tokio::sync::mpsc::UnboundedSender;

use super::Message;

const DEFAULT_LRCLIB_URL: &str = "https://lrclib.net";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongSource {
    System,
    ExternalLyri,
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
    pub(super) fn socket_addr(&self) -> Result<SocketAddr, String> {
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
    SongSource(SongSource),
    ExternalAddress(String),
    ExternalPort(u16),
    LrclibUrl(String),
    WebServerEnabled(bool),
    WebServerBindAddress(String),
    WebServerPort(u16),
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
            SettingsChange::SongSource(value) => self.song_source = value,
            SettingsChange::ExternalAddress(value) => self.external_lyri_address = value,
            SettingsChange::ExternalPort(value) => self.external_lyri_port = value,
            SettingsChange::LrclibUrl(value) => self.lrclib_server_url = value,
            SettingsChange::WebServerEnabled(value) => self.web_server_enabled = value,
            SettingsChange::WebServerBindAddress(value) => self.web_server_bind_address = value,
            SettingsChange::WebServerPort(value) => self.web_server_port = value,
            SettingsChange::WebServerAllowSeek(value) => self.web_server_allow_seek = value,
        }
    }
}

/// The settings pane's widgets whose state follows [`SettingsState`].
pub(super) struct SettingsPane {
    pub(super) root: gtk::Box,
    external: gtk::Box,
    web_fields: Vec<gtk::Widget>,
    web_error: gtk::Label,
}

impl SettingsPane {
    pub(super) fn new(settings: &SettingsState, tx: &UnboundedSender<Message>) -> Self {
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.set_tooltip_text(Some("Close settings"));
        let tx_close = tx.clone();
        close.connect_clicked(move |_| {
            let _ = tx_close.send(Message::CloseSettings);
        });
        let title = gtk::Label::new(Some("Settings"));
        title.add_css_class("settings-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let header = gtk::Box::new(Orientation::Horizontal, 12);
        header.append(&title);
        header.append(&close);

        let theme = gtk::Switch::new();
        if let Some(gtk_settings) = gtk::Settings::default() {
            gtk_settings
                .bind_property("gtk-application-prefer-dark-theme", &theme, "active")
                .bidirectional()
                .sync_create()
                .build();
        }

        let source = gtk::DropDown::from_strings(&["System", "External Lyri"]);
        source.set_selected(match settings.song_source {
            SongSource::System => 0,
            SongSource::ExternalLyri => 1,
        });
        let on_source = send(tx, SettingsChange::SongSource);
        source.connect_selected_notify(move |source| {
            on_source(match source.selected() {
                1 => SongSource::ExternalLyri,
                _ => SongSource::System,
            });
        });

        let external = column([
            labeled(
                "IP address",
                &entry(
                    &settings.external_lyri_address,
                    send(tx, SettingsChange::ExternalAddress),
                ),
            ),
            labeled(
                "Port",
                &port(
                    settings.external_lyri_port,
                    send(tx, SettingsChange::ExternalPort),
                ),
            ),
        ]);

        let web_enabled = switch(
            settings.web_server_enabled,
            send(tx, SettingsChange::WebServerEnabled),
        );
        let web_fields: Vec<gtk::Widget> = vec![
            labeled(
                "Bind address",
                &entry(
                    &settings.web_server_bind_address,
                    send(tx, SettingsChange::WebServerBindAddress),
                ),
            )
            .upcast(),
            labeled(
                "Port",
                &port(
                    settings.web_server_port,
                    send(tx, SettingsChange::WebServerPort),
                ),
            )
            .upcast(),
            labeled(
                "Allow viewers to seek",
                &switch(
                    settings.web_server_allow_seek,
                    send(tx, SettingsChange::WebServerAllowSeek),
                ),
            )
            .upcast(),
        ];
        let web_error = gtk::Label::new(None);
        web_error.add_css_class("settings-error");
        web_error.set_wrap(true);
        web_error.set_xalign(0.0);

        let web_rows = std::iter::once(labeled("Host local web view", &web_enabled).upcast())
            .chain(web_fields.iter().cloned())
            .chain([web_error.clone().upcast()]);

        let root = gtk::Box::new(Orientation::Vertical, 22);
        root.add_css_class("settings-pane");
        root.append(&header);
        root.append(&section(
            "Appearance",
            &labeled("Dark theme", &theme).upcast(),
        ));
        root.append(&section(
            "Song Source",
            &column([labeled("Source", &source), external.clone()]).upcast(),
        ));
        root.append(&section(
            "LRCLIB",
            &labeled(
                "Server URL",
                &entry(
                    &settings.lrclib_server_url,
                    send(tx, SettingsChange::LrclibUrl),
                ),
            )
            .upcast(),
        ));
        root.append(&section("Web Server", &column(web_rows).upcast()));

        let pane = Self {
            root,
            external,
            web_fields,
            web_error,
        };
        pane.sync(settings, None);
        pane
    }

    pub(super) fn sync(&self, settings: &SettingsState, web_server_error: Option<&str>) {
        self.external
            .set_visible(settings.song_source == SongSource::ExternalLyri);
        for field in &self.web_fields {
            field.set_sensitive(settings.web_server_enabled);
        }
        self.web_error
            .set_text(web_server_error.unwrap_or_default());
        self.web_error.set_visible(web_server_error.is_some());
    }
}

fn send<T: 'static>(
    tx: &UnboundedSender<Message>,
    change: fn(T) -> SettingsChange,
) -> impl Fn(T) + 'static {
    let tx = tx.clone();
    move |value| {
        let _ = tx.send(Message::Settings(change(value)));
    }
}

fn entry(text: &str, on_change: impl Fn(String) + 'static) -> gtk::Entry {
    let entry = gtk::Entry::builder().text(text).hexpand(true).build();
    entry.connect_changed(move |entry| on_change(entry.text().to_string()));
    entry
}

fn port(value: u16, on_change: impl Fn(u16) + 'static) -> gtk::SpinButton {
    let port = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
    port.set_value(f64::from(value));
    port.connect_value_changed(move |port| on_change(port.value_as_int() as u16));
    port
}

fn switch(active: bool, on_change: impl Fn(bool) + 'static) -> gtk::Switch {
    let switch = gtk::Switch::new();
    switch.set_active(active);
    switch.connect_active_notify(move |switch| on_change(switch.is_active()));
    switch
}

fn labeled(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    control.set_valign(Align::Center);

    let row = gtk::Box::new(Orientation::Horizontal, 12);
    row.add_css_class("settings-row");
    row.append(&label);
    row.append(control);
    row
}

fn column(children: impl IntoIterator<Item = impl IsA<gtk::Widget>>) -> gtk::Box {
    let column = gtk::Box::new(Orientation::Vertical, 10);
    for child in children {
        column.append(&child);
    }
    column
}

fn section(title: &str, content: &gtk::Widget) -> gtk::Box {
    let title = gtk::Label::new(Some(title));
    title.add_css_class("section-title");
    title.set_xalign(0.0);

    let section = gtk::Box::new(Orientation::Vertical, 8);
    section.append(&title);
    section.append(content);
    section
}
