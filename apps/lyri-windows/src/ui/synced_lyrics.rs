use std::time::Duration;

use windows_reactor::*;

use super::common::empty_state;
use crate::external_source::PlaybackControls;
use lyri_core::lyrics::{TimedLine, TrackKey, active_line_index, format_duration};

#[derive(Clone, PartialEq)]
pub(super) struct SyncedLyricsInput {
    pub(super) lines: Vec<TimedLine>,
    pub(super) position: Duration,
    /// Width the lines are laid out for; sizes the fonts.
    pub(super) viewport_width: f64,
    pub(super) seek: Option<LyricsSeekTarget>,
}

#[derive(Clone)]
pub(super) struct LyricsSeekTarget {
    pub(super) key: TrackKey,
    pub(super) controller: PlaybackControls,
}

impl PartialEq for LyricsSeekTarget {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

const LYRIC_SPACING: f64 = 16.0;
const LYRIC_EDGE_PADDING: f64 = 240.0;
const LYRIC_MIN_TEXT_WIDTH: f64 = 360.0;
const LINE_FADE: Duration = Duration::from_millis(180);

#[derive(Clone)]
pub(super) enum Message {
    View(ScrollViewMetrics),
    Seek(Duration),
}

/// Keeps the active lyric line centered in its scroll view.
pub(super) struct SyncedLyrics {
    view: ElementRef<ScrollView>,
    _observation: ElementObservation,
    /// Viewport height last reported by the scroll view, once it is bound.
    viewport_height: Option<f64>,
    scrolled_to: Option<f64>,
}

impl Component for SyncedLyrics {
    type Input = SyncedLyricsInput;
    type Message = Message;

    fn create(_: &SyncedLyricsInput, context: &ComponentContext<Self>) -> Self {
        let view = ElementRef::new();
        let observed = context.callback(Message::View);
        let observation = view.observe_view(move |metrics| observed.call(metrics));
        Self {
            view,
            _observation: observation,
            viewport_height: None,
            scrolled_to: None,
        }
    }

    fn input_changed(&mut self, input: &SyncedLyricsInput, _: &ComponentContext<Self>) {
        self.scroll_to_active(input);
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        let input = context.input();
        match message {
            Message::View(metrics) => {
                // Also reported for every scroll step; only a resize matters.
                if self.viewport_height != Some(metrics.viewport_height) {
                    self.viewport_height = Some(metrics.viewport_height);
                    self.scroll_to_active(input);
                }
            }
            Message::Seek(timestamp) => {
                if let Some(seek) = &input.seek {
                    seek.controller.seek_to(seek.key.clone(), timestamp);
                }
            }
        }
    }

    fn view(&self, input: &SyncedLyricsInput, context: &mut ViewContext<Self>) -> View {
        if input.lines.is_empty() {
            return empty_state("No timed lyric lines found.");
        }

        // None until the first line's timestamp: no line is highlighted yet.
        let active = active_line_index(&input.lines, input.position);
        let mut items: Vec<View> = Vec::with_capacity(input.lines.len() + 1);
        items.push(Border::new().height(LYRIC_EDGE_PADDING).into());
        items.extend(
            input
                .lines
                .iter()
                .enumerate()
                .map(|(index, line)| render_lyric_line(index, active, line, input, context)),
        );

        ScrollView::new()
            .element_ref(&self.view)
            .horizontal_scroll_bar_visibility(ScrollingScrollBarVisibility::Hidden)
            .vertical_scroll_bar_visibility(ScrollingScrollBarVisibility::Hidden)
            .content(
                StackPanel::new()
                    .spacing(LYRIC_SPACING)
                    .horizontal_alignment(HorizontalAlignment::Stretch)
                    .children(items),
            )
            .into()
    }
}

impl SyncedLyrics {
    fn scroll_to_active(&mut self, input: &SyncedLyricsInput) {
        let Some(height) = self.viewport_height else {
            return;
        };
        let active = active_line_index(&input.lines, input.position);
        let target = lyric_scroll_target(&input.lines, active, input.viewport_width, height);
        if self.scrolled_to == Some(target) {
            return;
        }
        if self.view.request_scroll_to(0.0, target, true, |_| {}) {
            self.scrolled_to = Some(target);
        }
    }
}

fn render_lyric_line(
    index: usize,
    active: Option<usize>,
    line: &TimedLine,
    input: &SyncedLyricsInput,
    context: &ViewContext<SyncedLyrics>,
) -> View {
    let viewport_width = input.viewport_width;
    let is_active = active == Some(index);
    // Before the first line starts, fade lines by distance from the top so
    // the upcoming lyrics are visible without highlighting any of them.
    let distance = active.map_or(index + 1, |active| index.abs_diff(active));
    let font_size = if is_active {
        responsive_active_font(viewport_width)
    } else {
        responsive_inactive_font(viewport_width)
    };
    let text_width = responsive_lyric_text_width(viewport_width, font_size);

    let opacity = match distance {
        0 => 1.0,
        1 => 0.58,
        2 => 0.28,
        _ => 0.1,
    };

    let line_container = Border::new()
        .min_height(responsive_min_line_height(viewport_width))
        .padding(Thickness::xy(0.0, 2.0))
        .opacity(opacity)
        .opacity_transition(LINE_FADE)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .content(
            TextBlock::new()
                .text(&line.text)
                .font_size(font_size)
                .font_weight(if is_active {
                    FontWeight::BOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_wrapping(TextWrapping::Wrap)
                .text_alignment(TextAlignment::Center)
                .max_width(text_width)
                .horizontal_alignment(HorizontalAlignment::Center),
        );

    if input.seek.is_none() {
        return line_container.into();
    }

    let timestamp = line.timestamp;
    line_container
        // A transparent background makes the whole row hit-testable.
        .background(Color::transparent())
        .on_pointer_released(context.callback(move |_: PointerEventInfo| Message::Seek(timestamp)))
        .tooltip(format!("Jump to {}", format_duration(timestamp)))
}

fn lyric_scroll_target(
    lines: &[TimedLine],
    active: Option<usize>,
    viewport_width: f64,
    viewport_height: f64,
) -> f64 {
    let anchor = active.unwrap_or(0).min(lines.len().saturating_sub(1));
    let child_top: f64 = LYRIC_EDGE_PADDING
        + LYRIC_SPACING
        + lines
            .iter()
            .take(anchor)
            .map(|line| estimated_lyric_height(line, false, viewport_width) + LYRIC_SPACING)
            .sum::<f64>();

    let child_height = lines
        .get(anchor)
        .map(|line| estimated_lyric_height(line, active.is_some(), viewport_width))
        .unwrap_or(0.0);
    let total_height = lyric_total_height(lines, active, viewport_width);
    let scrollable_height = (total_height - viewport_height).max(0.0);
    let target = child_top + child_height / 2.0 - viewport_height / 2.0;

    target.clamp(0.0, scrollable_height)
}

fn lyric_total_height(lines: &[TimedLine], active: Option<usize>, viewport_width: f64) -> f64 {
    let line_height = lines
        .iter()
        .enumerate()
        .map(|(index, line)| estimated_lyric_height(line, active == Some(index), viewport_width))
        .sum::<f64>();
    let gaps = lines.len() as f64;

    LYRIC_EDGE_PADDING + line_height + LYRIC_SPACING * gaps
}

fn estimated_lyric_height(line: &TimedLine, is_active: bool, viewport_width: f64) -> f64 {
    let font_size = if is_active {
        responsive_active_font(viewport_width)
    } else {
        responsive_inactive_font(viewport_width)
    };
    let text_width = responsive_lyric_text_width(viewport_width, font_size);
    let average_char_width = font_size * 0.52;
    let chars_per_line = (text_width / average_char_width).floor().max(8.0);
    let char_count = line.text.chars().count().max(1) as f64;
    let wrapped_lines = (char_count / chars_per_line).ceil().max(1.0);

    (wrapped_lines * font_size * 1.32 + 4.0).max(responsive_min_line_height(viewport_width))
}

fn responsive_lyric_text_width(viewport_width: f64, font_size: f64) -> f64 {
    let active_width = responsive_active_lyric_text_width(viewport_width);
    let active_font = responsive_active_font(viewport_width);

    active_width * font_size / active_font
}

fn responsive_active_lyric_text_width(viewport_width: f64) -> f64 {
    (viewport_width * 0.94).max(LYRIC_MIN_TEXT_WIDTH)
}

fn responsive_active_font(viewport_width: f64) -> f64 {
    if viewport_width >= 1400.0 { 38.0 } else { 28.0 }
}

fn responsive_inactive_font(viewport_width: f64) -> f64 {
    if viewport_width >= 1400.0 { 26.0 } else { 20.0 }
}

fn responsive_min_line_height(viewport_width: f64) -> f64 {
    if viewport_width >= 1400.0 { 72.0 } else { 56.0 }
}
