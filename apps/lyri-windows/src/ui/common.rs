use windows_reactor::*;

pub(super) fn empty_state(message: &str) -> View {
    centered_stack(
        TextBlock::new()
            .text(message)
            .font_size(18.0)
            .opacity(0.68)
            .text_wrapping(TextWrapping::Wrap),
    )
}

pub(super) fn centered_stack(children: impl Into<View>) -> View {
    Border::new()
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .content(children)
        .into()
}
