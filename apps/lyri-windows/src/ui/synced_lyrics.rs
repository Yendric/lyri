use std::time::Duration;

use windows_reactor::*;

use super::common::empty_state;
use crate::external_source::PlaybackControls;
use lyri_core::lyrics::{TimedLine, TrackKey, active_line_index, format_duration};

#[derive(Clone, PartialEq)]
pub(super) struct SyncedLyricsProps {
    pub(super) lines: Vec<TimedLine>,
    pub(super) position: Duration,
    pub(super) viewport_height: f64,
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

pub(super) fn synced_lyrics(props: &SyncedLyricsProps, cx: &mut RenderCx) -> Element {
    if props.lines.is_empty() {
        return empty_state("No timed lyric lines found.");
    }

    // None until the first line's timestamp: no line is highlighted yet.
    let active = active_line_index(&props.lines, props.position);

    let scroll_handle = cx.use_ref(Option::<ScrollViewHandle>::None);
    let (scroll_ready, set_scroll_ready) = cx.use_state(false);
    let active_index = cx.use_ref(active);
    let line_count = props.lines.len();
    let target_offset = lyric_scroll_target(
        &props.lines,
        active,
        props.viewport_width,
        props.viewport_height,
    );
    *active_index.borrow_mut() = active;

    let handle_for_effect = scroll_handle.clone();
    cx.use_effect(
        (scroll_ready, active, line_count, target_offset),
        move || {
            if let Some(handle) = handle_for_effect.borrow().as_ref() {
                let _ = handle.scroll_to_vertical_offset(target_offset);
            }
        },
    );

    let lyric_items = props
        .lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            render_lyric_line(
                index,
                active,
                line,
                props.viewport_width,
                props.seek.clone(),
            )
        })
        .collect::<Vec<Element>>();

    let mut items = Vec::with_capacity(lyric_items.len() + 1);
    items.push(spacer(LYRIC_EDGE_PADDING));
    items.extend(lyric_items);

    let list_stack = vstack(items)
        .spacing(LYRIC_SPACING)
        .horizontal_alignment(HorizontalAlignment::Stretch);

    let handle_for_ready = scroll_handle.clone();
    let handle_for_resize = scroll_handle.clone();
    let active_for_resize = active_index.clone();
    let lines_for_resize = props.lines.clone();
    scroll_view(list_stack)
        .on_ready(move |handle| {
            *handle_for_ready.borrow_mut() = Some(handle);
            set_scroll_ready.call(true);
        })
        .on_resize(move |width, height| {
            let active = *active_for_resize.borrow();
            let target_offset = lyric_scroll_target(&lines_for_resize, active, width, height);
            if let Some(handle) = handle_for_resize.borrow().as_ref() {
                let _ = handle.scroll_to_vertical_offset(target_offset);
            }
        })
        .horizontal_scroll_bar_visibility(ScrollViewScrollBarVisibility::Hidden)
        .vertical_scroll_bar_visibility(ScrollViewScrollBarVisibility::Hidden)
        .height(props.viewport_height)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .vertical_alignment(VerticalAlignment::Stretch)
        .into()
}

fn spacer(height: f64) -> Element {
    border(Element::Empty).height(height).into()
}

fn render_lyric_line(
    index: usize,
    active: Option<usize>,
    line: &TimedLine,
    viewport_width: f64,
    seek: Option<LyricsSeekTarget>,
) -> Element {
    let is_active = active == Some(index);
    // Before the first line starts, fade lines by distance from the top so
    // the upcoming lyrics are visible without highlighting any of them.
    let distance = active.map_or(index + 1, |active| index.abs_diff(active));
    let active_font = responsive_active_font(viewport_width);
    let inactive_font = responsive_inactive_font(viewport_width);
    let font_size = if is_active {
        active_font
    } else {
        inactive_font
    };
    let text_width = responsive_lyric_text_width(viewport_width, font_size);

    let opacity = match distance {
        0 => 1.0,
        1 => 0.58,
        2 => 0.28,
        _ => 0.1,
    };

    let mut line_container = border(
        text_block(&line.text)
            .font_size(font_size)
            .font_weight(if is_active { 700 } else { 500 })
            .wrap()
            .text_alignment(TextAlignment::Center)
            .max_width(text_width)
            .horizontal_alignment(HorizontalAlignment::Center),
    )
    .min_height(responsive_min_line_height(viewport_width))
    .padding(Thickness::xy(0.0, 2.0))
    .opacity(opacity)
    .horizontal_alignment(HorizontalAlignment::Stretch)
    .with_opacity_transition(Duration::from_millis(180));

    if let Some(seek) = seek {
        let key = seek.key;
        let controller = seek.controller;
        let timestamp = line.timestamp;
        line_container = line_container
            .background(Color {
                a: 0,
                r: 0,
                g: 0,
                b: 0,
            })
            .tooltip(format!("Jump to {}", format_duration(timestamp)))
            .on_tapped(move || {
                controller.seek_to(key.clone(), timestamp);
            });
    }

    line_container.into()
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
