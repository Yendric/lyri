use windows_reactor::*;

pub(super) fn empty_state(message: &str) -> Element {
    centered_stack(text_block(message).font_size(18.0).opacity(0.68).wrap())
}

pub(super) fn centered_stack(children: impl Into<Element>) -> Element {
    border(children)
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .into()
}
