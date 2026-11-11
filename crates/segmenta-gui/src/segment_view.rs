//! `SegmentaSegmentView`: one block per engine segment.
//!
//! The engine splits ranges in hot, so the bar must show every range with its
//! own proportional width and its own filled part. The geometry lives in the
//! pure [`layout_segments`] function; `snapshot()` only paints the fractions.

use std::cell::{Cell, RefCell};

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::SegmentItem;

/// One block of the bar, in fractions of the widget width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentBlock {
    /// Left edge, `0.0..=1.0`.
    pub start: f64,
    /// Block width, `0.0..=1.0`.
    pub width: f64,
    /// Already downloaded part of the block, `0.0..=1.0`.
    pub filled: f64,
}

/// Geometry of the segment bar. Pure so it can be tested without a display.
///
/// `total_bytes` is the file size; when it is still unknown the spans of the
/// segments themselves are used as the denominator, which keeps the bar
/// proportional and division-safe.
pub fn layout_segments(segments: &[SegmentItem], total_bytes: u64) -> Vec<SegmentBlock> {
    if segments.is_empty() {
        return Vec::new();
    }
    let owned: u64 = segments.iter().map(SegmentItem::span).sum();
    let total = if total_bytes > 0 { total_bytes } else { owned };
    if total == 0 {
        return Vec::new();
    }

    let denominator = total as f64;
    let mut cursor = 0.0f64;
    let mut blocks = Vec::with_capacity(segments.len());
    for segment in segments {
        let span = segment.span();
        let width = (span as f64 / denominator).clamp(0.0, 1.0);
        let filled = if span == 0 {
            0.0
        } else {
            (segment.downloaded() as f64 / span as f64).clamp(0.0, 1.0)
        };
        blocks.push(SegmentBlock {
            start: cursor,
            width,
            filled,
        });
        cursor = (cursor + width).min(1.0);
    }

    // Absorb floating point drift into the last block so the bar ends at 1.0.
    if let Some(last) = blocks.last_mut() {
        let end = last.start + last.width;
        if (end - 1.0).abs() < 1e-6 {
            last.width = (1.0 - last.start).max(0.0);
        }
    }
    blocks
}

/// Fallback accent used only when the theme color is not available yet (for
/// example while the widget is not realized). Every other color comes from CSS.
const FALLBACK_ACCENT: (f32, f32, f32, f32) = (0.21, 0.52, 0.89, 1.0);

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SegmentaSegmentView {
        pub(super) segments: RefCell<Vec<SegmentItem>>,
        pub(super) total_bytes: Cell<u64>,
        pub(super) accessible: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SegmentaSegmentView {
        const NAME: &'static str = "SegmentaSegmentView";
        type Type = super::SegmentaSegmentView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SegmentaSegmentView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_overflow(gtk::Overflow::Hidden);
            obj.set_hexpand(true);
            obj.set_valign(gtk::Align::Center);
            // `.accent` makes `WidgetExt::color()` return the system accent
            // color, so the bar follows the desktop accent without hardcoding.
            obj.add_css_class("accent");
            obj.update_property(&[gtk::accessible::Property::Label("Segmentos de la descarga")]);
        }
    }

    impl WidgetImpl for SegmentaSegmentView {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => (10, 10, -1, -1),
                _ => (48, 48, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let width = self.obj().width() as f32;
            let height = self.obj().height() as f32;
            if width <= 0.0 || height <= 0.0 {
                return;
            }

            let accent = self.obj().color();
            let accent = if accent.alpha() <= 0.0 {
                let (red, green, blue, alpha) = FALLBACK_ACCENT;
                gdk::RGBA::new(red, green, blue, alpha)
            } else {
                accent
            };
            let track = gdk::RGBA::new(accent.red(), accent.green(), accent.blue(), 0.18);

            let blocks = layout_segments(&self.segments.borrow(), self.total_bytes.get());
            if blocks.is_empty() {
                snapshot.append_color(&track, &graphene::Rect::new(0.0, 0.0, width, height));
                return;
            }

            let gap = if blocks.len() > 1 {
                (width / blocks.len() as f32 * 0.06).clamp(0.5, 3.0)
            } else {
                0.0
            };
            for block in blocks {
                let left = block.start as f32 * width + gap / 2.0;
                let full = block.width as f32 * width - gap;
                if full <= 0.0 {
                    continue;
                }
                let filled = (full * block.filled as f32).clamp(0.0, full);
                snapshot.append_color(&track, &graphene::Rect::new(left, 0.0, full, height));
                if filled > 0.0 {
                    snapshot.append_color(&accent, &graphene::Rect::new(left, 0.0, filled, height));
                }
            }
        }
    }
}

glib::wrapper! {
    /// Bar with one block per segment of a download.
    pub struct SegmentaSegmentView(ObjectSubclass<imp::SegmentaSegmentView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SegmentaSegmentView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SegmentaSegmentView {
    /// Replaces the painted segments.
    pub fn set_segments(&self, segments: &[SegmentItem], total_bytes: u64) {
        let imp = self.imp();
        let active = segments
            .iter()
            .filter(|segment| segment.is_active())
            .count();
        let label = if segments.is_empty() {
            "Sin segmentos todavía".to_string()
        } else {
            format!("{active} de {} segmentos activos", segments.len())
        };
        *imp.accessible.borrow_mut() = label.clone();
        *imp.segments.borrow_mut() = segments.to_vec();
        imp.total_bytes.set(total_bytes);
        self.update_property(&[gtk::accessible::Property::Label(&label)]);
        self.queue_draw();
    }

    /// Number of blocks currently painted.
    pub fn segments(&self) -> usize {
        self.imp().segments.borrow().len()
    }

    /// Number of segments still transferring.
    pub fn active_segments(&self) -> usize {
        self.imp()
            .segments
            .borrow()
            .iter()
            .filter(|segment| segment.is_active())
            .count()
    }

    /// Accessible label, also exposed for tests.
    pub fn accessible_label(&self) -> String {
        self.imp().accessible.borrow().clone()
    }
}
