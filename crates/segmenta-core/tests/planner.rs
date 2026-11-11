//! Invariants of the initial segmentation and of the hot-split arithmetic.
//!
//! These tests are pure: no sockets, no files. Specs S4 and S5.

use segmenta_core::planner::{
    DonorView, MIN_SPLIT_SIZE, SegmentRange, SplitOutcome, SplitReject, choose_donor, clamp_read,
    connections_for, initial_segments, midpoint_of, remaining, segments_for, split_at,
};

const MB: u64 = 1024 * 1024;

#[test]
fn connection_count_follows_size_thresholds() {
    assert_eq!(connections_for(1, 16), 1);
    assert_eq!(connections_for(5 * MB - 1, 16), 1);
    assert_eq!(connections_for(5 * MB, 16), 4);
    assert_eq!(connections_for(50 * MB - 1, 16), 4);
    assert_eq!(connections_for(50 * MB, 16), 16);
    assert_eq!(connections_for(2 * 1024 * MB, 32), 32);
    // The user request is capped at 32 connections.
    assert_eq!(connections_for(2 * 1024 * MB, 64), 32);
    assert_eq!(connections_for(2 * 1024 * MB, 0), 1);
}

#[test]
fn requested_segments_are_contiguous_and_cover_the_file() {
    let sizes = [
        1u64, 2, 3, 4, 7, MB, 5 * MB - 1, 5 * MB, 5 * MB + 1, 50 * MB - 1, 50 * MB, 50 * MB + 7,
        7 * 1024 * MB + 3,
    ];
    for total in sizes {
        for requested in [1u32, 2, 3, 4, 7, 16, 32] {
            for segments in [segments_for(total, requested), initial_segments(total, requested)] {
                assert!(!segments.is_empty(), "total={total} requested={requested}");
                assert_eq!(segments[0].start, 0, "first segment must start at 0");
                assert_eq!(
                    segments.last().unwrap().end,
                    total - 1,
                    "last segment must end at total-1 (total={total})"
                );
                for pair in segments.windows(2) {
                    assert_eq!(
                        pair[0].end + 1,
                        pair[1].start,
                        "gap or overlap at total={total} requested={requested}: {pair:?}"
                    );
                }
                let covered: u64 = segments.iter().map(|s| s.len()).sum();
                assert_eq!(covered, total, "coverage mismatch for total={total}");
                assert!(!segments.iter().any(|s| s.is_empty()), "empty segment");
            }
        }
    }
}

#[test]
fn initial_segmentation_respects_the_size_thresholds() {
    assert_eq!(initial_segments(4 * MB, 16).len(), 1);
    assert_eq!(initial_segments(5 * MB, 16).len(), 4);
    assert_eq!(initial_segments(50 * MB, 16).len(), 16);
    assert_eq!(initial_segments(50 * MB, 8).len(), 8);
    assert_eq!(initial_segments(50 * MB, 99).len(), 32);
}

#[test]
fn requested_segments_split_evenly_without_off_by_one() {
    assert_eq!(
        segments_for(1000, 4),
        vec![
            SegmentRange { start: 0, end: 249 },
            SegmentRange { start: 250, end: 499 },
            SegmentRange { start: 500, end: 749 },
            SegmentRange { start: 750, end: 999 },
        ]
    );
}

#[test]
fn tiny_files_never_produce_empty_segments() {
    let segments = segments_for(3, 4);
    assert_eq!(segments.len(), 3);
    assert_eq!(segments.iter().map(|s| s.len()).sum::<u64>(), 3);
    assert!(segments.iter().all(|s| s.len() == 1));
    assert!(segments_for(0, 8).is_empty());
    // A sub-5 MB file keeps a single connection by policy.
    assert_eq!(initial_segments(3, 8).len(), 1);
}

#[test]
fn remaining_and_read_clamp_use_inclusive_ranges() {
    assert_eq!(remaining(9999, 3000), 7000);
    assert_eq!(remaining(9999, 10_000), 0);
    // Reading must never run past the segment end.
    assert_eq!(clamp_read(9990, 9999, 65536), 10);
    assert_eq!(clamp_read(0, 9999, 4096), 4096);
    assert_eq!(clamp_read(10_000, 9999, 4096), 0);
}

#[test]
fn split_midpoint_is_the_middle_of_the_remaining_region() {
    // OpenSpec 10 uses [0,9999] with next_offset 3000 and midpoint 6500.
    assert_eq!(midpoint_of(9999, 3000), 6500);
    assert_eq!(midpoint_of(9999, 9000), 9500);
}

#[test]
fn safe_hot_split_invariants() {
    let donor = SegmentRange { start: 0, end: 9999 };
    let outcome = split_at(donor, 3000, 6500);
    let SplitOutcome::Accepted {
        donor: new_donor,
        new_segment,
        clamped_next,
    } = outcome
    else {
        panic!("split must be accepted: {outcome:?}");
    };
    assert_eq!(new_donor, SegmentRange { start: 0, end: 6500 });
    assert_eq!(new_segment, SegmentRange { start: 6501, end: 9999 });
    assert_eq!(clamped_next, 6501);
    assert_eq!(new_donor.end + 1, new_segment.start, "no gap, no overlap");
    assert_eq!(new_segment.end, donor.end, "the new segment inherits the end");
}

#[test]
fn split_rejects_a_donor_that_already_passed_the_midpoint() {
    let donor = SegmentRange { start: 0, end: 9999 };
    assert_eq!(
        split_at(donor, 7000, 6500),
        SplitOutcome::Rejected(SplitReject::DonorAlreadyPast)
    );
    // A midpoint at or beyond the donor end would create an empty segment.
    assert_eq!(
        split_at(donor, 0, 9999),
        SplitOutcome::Rejected(SplitReject::MidpointNotSplittable)
    );
}

#[test]
fn donor_selection_picks_the_largest_remaining_above_threshold() {
    let donors = vec![
        DonorView { id: 1, start: 0, end: 10 * MB, next: 10 * MB },
        DonorView { id: 2, start: 10 * MB + 1, end: 30 * MB, next: 10 * MB + 1 },
        DonorView { id: 3, start: 30 * MB + 1, end: 31 * MB, next: 30 * MB + 1 },
    ];
    let choice = choose_donor(&donors).expect("a donor is available");
    assert_eq!(choice.id, 2);
    assert_eq!(choice.midpoint, 10 * MB + 1 + (20 * MB) / 2);
    assert_eq!(choice.clamped_next, choice.midpoint + 1);
}

#[test]
fn donor_selection_refuses_small_remainders() {
    let donors = vec![DonorView {
        id: 1,
        start: 0,
        end: MIN_SPLIT_SIZE - 2,
        next: 0,
    }];
    assert!(choose_donor(&donors).is_none());
}
