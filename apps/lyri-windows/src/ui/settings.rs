use windows_reactor::*;

pub(super) const DEFAULT_LRCLIB_URL: &str = "https://lrclib.net";

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

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SettingsState {
    song_source: SongSource,
    external_lyri_address: String,
    external_lyri_port: f64,
    lrclib_server_url: String,
    web_server_enabled: bool,
    web_server_bind_address: String,
    web_server_port: f64,
    web_server_allow_seek: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            song_source: SongSource::System,
            external_lyri_address: "127.0.0.1".to_string(),
            external_lyri_port: 8080.0,
            lrclib_server_url: DEFAULT_LRCLIB_URL.to_string(),
            web_server_enabled: false,
            web_server_bind_address: "0.0.0.0".to_string(),
            web_server_port: 8080.0,
            web_server_allow_seek: false,
        }
    }
}

pub(super) fn render_settings_pane(
    settings: SettingsState,
    set_settings: SetState<SettingsState>,
    is_dark: bool,
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

    let source_state = settings.clone();
    let source_setter = set_settings.clone();
    let source_picker = ComboBox::new(["System", "External Lyri"])
        .header("Source")
        .selected_index(settings.song_source.selected_index())
        .on_selection_changed(move |index| {
            let mut next = source_state.clone();
            next.song_source = SongSource::from_selected_index(index);
            source_setter.call(next);
        });

    let mut song_source_items = vec![Element::from(source_picker)];
    if settings.song_source == SongSource::ExternalLyri {
        let external_address_state = settings.clone();
        let external_address_setter = set_settings.clone();
        let external_address = text_box(settings.external_lyri_address.clone())
            .header("IP address")
            .placeholder("127.0.0.1")
            .on_changed(move |value| {
                let mut next = external_address_state.clone();
                next.external_lyri_address = value;
                external_address_setter.call(next);
            });

        let external_port_state = settings.clone();
        let external_port_setter = set_settings.clone();
        let external_port = NumberBox::new(settings.external_lyri_port)
            .header("Port")
            .range(1.0, 65535.0)
            .on_value_changed(move |value| {
                let mut next = external_port_state.clone();
                next.external_lyri_port = value.round().clamp(1.0, 65535.0);
                external_port_setter.call(next);
            });

        song_source_items.push(Element::from(external_address));
        song_source_items.push(Element::from(external_port));
    }

    let song_source = settings_section(
        "Song Source",
        vstack(song_source_items)
            .spacing(14.0)
            .horizontal_alignment(HorizontalAlignment::Stretch),
    );

    let lrclib_url_for_change = settings.clone();
    let lrclib_setter = set_settings.clone();
    let lrclib = settings_section(
        "LRCLIB",
        text_box(settings.lrclib_server_url.clone())
            .header("Server URL")
            .placeholder(DEFAULT_LRCLIB_URL)
            .on_changed(move |value| {
                let mut next = lrclib_url_for_change.clone();
                next.lrclib_server_url = value;
                lrclib_setter.call(next);
            }),
    );

    let web_enabled = settings.web_server_enabled;
    let web_enabled_state = settings.clone();
    let web_enabled_setter = set_settings.clone();
    let host_toggle = ToggleSwitch::new(web_enabled)
        .header("Host local web view")
        .on_content("On")
        .off_content("Off")
        .on_changed(move |value| {
            let mut next = web_enabled_state.clone();
            next.web_server_enabled = value;
            web_enabled_setter.call(next);
        });

    let bind_state = settings.clone();
    let bind_setter = set_settings.clone();
    let bind_address = text_box(settings.web_server_bind_address.clone())
        .header("Bind address")
        .placeholder("0.0.0.0")
        .enabled(web_enabled)
        .on_changed(move |value| {
            let mut next = bind_state.clone();
            next.web_server_bind_address = value;
            bind_setter.call(next);
        });

    let port_state = settings.clone();
    let port_setter = set_settings.clone();
    let port = NumberBox::new(settings.web_server_port)
        .header("Port")
        .range(1.0, 65535.0)
        .enabled(web_enabled)
        .on_value_changed(move |value| {
            let mut next = port_state.clone();
            next.web_server_port = value.round().clamp(1.0, 65535.0);
            port_setter.call(next);
        });

    let seek_state = settings.clone();
    let seek_setter = set_settings;
    let allow_seek = ToggleSwitch::new(settings.web_server_allow_seek)
        .header("Allow viewers to seek")
        .on_content("Allowed")
        .off_content("View only")
        .enabled(web_enabled)
        .on_changed(move |value| {
            let mut next = seek_state.clone();
            next.web_server_allow_seek = value;
            seek_setter.call(next);
        });

    let web_server = settings_section(
        "Web Server",
        vstack((host_toggle, bind_address, port, allow_seek))
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
