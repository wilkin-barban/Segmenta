//! Pure arithmetic of `SegmentaSegmentView`.
//!
//! The widget itself needs a display, but the geometry must be testable without
//! one: `snapshot()` only converts these fractions into rectangles.

use serde_json::json;

use segmenta_gui::model::SegmentItem;
use segmenta_gui::segment_view::{layout_segments, SegmentBlock};

fn segment(id: u32, start: u64, end: u64, next: u64) -> SegmentItem {
    serde_json::from_value(json!({
        "id": id,
        "start_offset": start,
        "end_offset": end,
        "next_offset": next,
        "status": "active",
        "speed_bps": 0
    }))
    .expect("segment parses")
}

fn even_segments(count: u32, total: u64) -> Vec<SegmentItem> {
    let span = total / u64::from(count);
    (0..count)
        .map(|index| {
            let start = u64::from(index) * span;
            let end = if index + 1 == count {
                total - 1
            } else {
                start + span - 1
            };
            segment(index, start, end, start)
        })
        .collect()
}

fn total_width(blocks: &[SegmentBlock]) -> f64 {
    blocks.iter().map(|block| block.width).sum()
}

#[test]
fn one_block_fills_the_whole_bar() {
    let blocks = layout_segments(&even_segments(1, 1000), 1000);
    assert_eq!(blocks.len(), 1);
    assert!((blocks[0].start - 0.0).abs() < 1e-9);
    assert!((blocks[0].width - 1.0).abs() < 1e-9);
    assert!(blocks[0].filled.abs() < 1e-9);
}

#[test]
fn sixteen_and_thirty_two_blocks_stay_proportional() {
    for count in [16u32, 32u32] {
        let blocks = layout_segments(&even_segments(count, 1_000_000), 1_000_000);
        assert_eq!(blocks.len() as u32, count);
        assert!(
            (total_width(&blocks) - 1.0).abs() < 1e-6,
            "{count} blocks cover {}",
            total_width(&blocks)
        );
        for (index, block) in blocks.iter().enumerate() {
            assert!(block.width > 0.0 && block.width.is_finite());
            assert!(block.start >= 0.0 && block.start <= 1.0);
            assert!((block.width - 1.0 / f64::from(count)).abs() < 1e-6);
            assert!((block.start - index as f64 / f64::from(count)).abs() < 1e-6);
        }
    }
}

#[test]
fn filled_fraction_follows_next_offset() {
    let mut segments = even_segments(2, 1000);
    // First block: half of its span is already on disk.
    segments[0].next_offset = 250;
    // Second block: completed, and a stray offset beyond the end must clamp.
    segments[1].next_offset = 9_999;
    let blocks = layout_segments(&segments, 1000);
    assert_eq!(blocks.len(), 2);
    assert!(
        (blocks[0].filled - 0.5).abs() < 1e-9,
        "{}",
        blocks[0].filled
    );
    assert!(
        (blocks[1].filled - 1.0).abs() < 1e-9,
        "{}",
        blocks[1].filled
    );
}

#[test]
fn unknown_total_falls_back_to_the_segment_spans() {
    let blocks = layout_segments(&even_segments(4, 4000), 0);
    assert_eq!(blocks.len(), 4);
    assert!((total_width(&blocks) - 1.0).abs() < 1e-6);
}

#[test]
fn empty_input_has_no_blocks() {
    assert!(layout_segments(&[], 0).is_empty());
    // A single zero-length segment must not divide by zero.
    let degenerate = vec![segment(0, 0, 0, 0)];
    let blocks = layout_segments(&degenerate, 0);
    assert!(blocks
        .iter()
        .all(|block| block.width.is_finite() && block.start.is_finite()));
}
