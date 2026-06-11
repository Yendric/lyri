use std::time::Duration;

use windows_reactor::*;

pub(super) fn empty_state(message: &str) -> Element {
    centered_stack(text_block(message).font_size(18.0).opacity(0.68).wrap())
}

pub(super) fn centered_stack(children: impl Into<Element>) -> Element {
    border(children)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .vertical_alignment(VerticalAlignment::Stretch)
        .into()
}

pub(super) fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}
