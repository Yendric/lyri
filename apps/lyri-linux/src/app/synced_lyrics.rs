use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib::{ControlFlow, Propagation};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use tokio::sync::mpsc::UnboundedSender;

use lyri_core::lyrics::{TimedLine, active_line_index, format_duration};

use super::Message;

const SCROLL_DURATION_US: f64 = 220_000.0;
const DISTANCE_CLASSES: [&str; 4] = ["active", "near", "far", "distant"];

/// Keeps the active lyric line centered until the user scrolls themselves.
#[derive(Clone)]
pub(super) struct SyncedLyrics(Rc<Inner>);

struct Inner {
    root: gtk::Overlay,
    scrolled: gtk::ScrolledWindow,
    lines: Vec<gtk::Label>,
    back: gtk::Button,
    active: Cell<Option<usize>>,
    following: Cell<bool>,
    scroll: RefCell<Option<gtk::TickCallbackId>>,
}

impl SyncedLyrics {
    /// `tx` is set when lines can be clicked to seek.
    pub(super) fn new(
        lines: &[TimedLine],
        position: Duration,
        tx: Option<&UnboundedSender<Message>>,
    ) -> Self {
        let list = gtk::Box::new(Orientation::Vertical, 16);
        list.add_css_class("lyrics-list");
        let labels: Vec<gtk::Label> = lines.iter().map(|line| lyric_line(line, tx)).collect();
        for label in &labels {
            list.append(label);
        }

        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .build();

        let back = gtk::Button::with_label("Back to current line");
        back.add_css_class("suggested-action");
        back.add_css_class("pill");
        back.set_halign(Align::Center);
        back.set_valign(Align::End);
        back.set_margin_bottom(20);
        back.set_visible(false);

        let root = gtk::Overlay::new();
        root.set_child(Some(&scrolled));
        root.add_overlay(&back);

        let view = Self(Rc::new(Inner {
            root,
            scrolled,
            lines: labels,
            back,
            active: Cell::new(None),
            following: Cell::new(true),
            scroll: RefCell::new(None),
        }));
        view.connect_handlers();
        view.set_active(active_line_index(lines, position), false);
        view
    }

    pub(super) fn widget(&self) -> &gtk::Overlay {
        &self.0.root
    }

    pub(super) fn update(&self, lines: &[TimedLine], position: Duration) {
        let active = active_line_index(lines, position);
        if active != self.0.active.get() {
            self.set_active(active, true);
        }
    }

    pub(super) fn follow(&self) {
        self.0.following.set(true);
        self.0.back.set_visible(false);
        self.scroll_to_active(true);
    }

    fn connect_handlers(&self) {
        let weak = Rc::downgrade(&self.0);
        let back_weak = weak.clone();
        self.0.back.connect_clicked(move |_| {
            with(&back_weak, SyncedLyrics::follow);
        });

        // Wheel and touchpad input only; our own scrolling never reaches it.
        let user_scroll =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        user_scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let scroll_weak = weak.clone();
        user_scroll.connect_scroll(move |_, _, _| {
            with(&scroll_weak, SyncedLyrics::unfollow);
            Propagation::Proceed
        });
        self.0.scrolled.add_controller(user_scroll);

        // Resizes and rewraps move the active line; re-center unless animating.
        self.0.scrolled.vadjustment().connect_changed(move |_| {
            with(&weak, |view| {
                if view.0.scroll.borrow().is_none() {
                    view.scroll_to_active(false);
                }
            });
        });
    }

    fn unfollow(&self) {
        self.0.following.set(false);
        self.0.back.set_visible(true);
        self.cancel_scroll();
    }

    fn set_active(&self, active: Option<usize>, animate: bool) {
        self.0.active.set(active);
        for (index, label) in self.0.lines.iter().enumerate() {
            // Before the first line starts, fade by distance from the top
            // without highlighting any line.
            let distance = active.map_or(index + 1, |active| index.abs_diff(active));
            let class = DISTANCE_CLASSES[distance.min(DISTANCE_CLASSES.len() - 1)];
            for other in DISTANCE_CLASSES {
                if other != class {
                    label.remove_css_class(other);
                }
            }
            label.add_css_class(class);
        }
        self.scroll_to_active(animate);
    }

    fn cancel_scroll(&self) {
        if let Some(id) = self.0.scroll.take() {
            id.remove();
        }
    }

    /// Scrolls on frame ticks, so it waits for layout and tracks the line as
    /// font transitions resize it.
    fn scroll_to_active(&self, animate: bool) {
        self.cancel_scroll();
        if !self.0.following.get() {
            return;
        }
        let anchor = self.0.active.get().unwrap_or(0);
        let Some(line) = self.0.lines.get(anchor).cloned() else {
            return;
        };

        let weak = Rc::downgrade(&self.0);
        let start: Cell<Option<(f64, i64)>> = Cell::new(None);
        let id = self.0.scrolled.add_tick_callback(move |_, clock| {
            let Some(inner) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            let adjustment = inner.scrolled.vadjustment();
            let page = adjustment.page_size();
            // Measured in the viewport, as list coordinates exclude its padding.
            let Some(bounds) = line.compute_bounds(&inner.scrolled).filter(|_| page > 0.0) else {
                return ControlFlow::Continue;
            };

            let max = (adjustment.upper() - page).max(0.0);
            let center = adjustment.value() + f64::from(bounds.y() + bounds.height() / 2.0);
            let target = (center - page / 2.0).clamp(0.0, max);
            let (from, began) = start
                .get()
                .unwrap_or((adjustment.value(), clock.frame_time()));
            start.set(Some((from, began)));
            let progress = if animate {
                ((clock.frame_time() - began) as f64 / SCROLL_DURATION_US).min(1.0)
            } else {
                1.0
            };
            let eased = 1.0 - (1.0 - progress).powi(3);
            adjustment.set_value(from + (target - from) * eased);

            if progress < 1.0 {
                return ControlFlow::Continue;
            }
            inner.scroll.replace(None);
            ControlFlow::Break
        });
        self.0.scroll.replace(Some(id));
    }
}

fn with(weak: &Weak<Inner>, f: impl FnOnce(&SyncedLyrics)) {
    if let Some(inner) = weak.upgrade() {
        f(&SyncedLyrics(inner));
    }
}

fn lyric_line(line: &TimedLine, tx: Option<&UnboundedSender<Message>>) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(&line.text)
        .wrap(true)
        // Long words may break too, so no line can widen the window.
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        // Full row width: centered wrapping labels get squeezed by GTK.
        .justify(gtk::Justification::Center)
        .xalign(0.5)
        .css_classes(["lyric-line"])
        .build();

    if let Some(tx) = tx {
        let timestamp = line.timestamp;
        let tx = tx.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let _ = tx.send(Message::Seek(timestamp));
        });
        label.add_controller(click);
        label.add_css_class("seekable");
        label.set_cursor_from_name(Some("pointer"));
        label.set_tooltip_text(Some(&format!("Jump to {}", format_duration(timestamp))));
    }
    label
}
