//! Segmentation arithmetic: initial planning and cooperative hot-split.
//!
//! Ranges are inclusive `[start, end]`; `next` is the next byte to write, so an
//! authored range carries `start <= next <= end + 1`, exactly like `segments.next_offset`
//! in the normative DDL.

/// Bytes required for a split to be worth a new TLS/TCP handshake.
pub const MIN_SPLIT_SIZE: u64 = 2 * 1024 * 1024;

const MB: u64 = 1024 * 1024;

/// One inclusive byte range owned by a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentRange {
    /// First byte of the range.
    pub start: u64,
    /// Last byte of the range, inclusive.
    pub end: u64,
}

impl SegmentRange {
    /// Number of bytes in the range.
    pub fn len(&self) -> u64 {
        if self.end < self.start {
            0
        } else {
            self.end - self.start + 1
        }
    }

    /// Whether the range covers no byte at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// First byte after the range.
    pub fn next(&self) -> u64 {
        self.end.saturating_add(1)
    }
}

/// Attempted split of one donor range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitOutcome {
    /// The donor hands `[midpoint+1, end]` to a new worker.
    Accepted {
        /// The donor's remaining range.
        donor: SegmentRange,
        /// The new worker's range.
        new_segment: SegmentRange,
        /// First byte the new worker requests (`midpoint + 1`).
        clamped_next: u64,
    },
    /// The donor cannot be split; the coordinator must pick another one.
    Rejected(SplitReject),
}

/// Why a donor refused a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitReject {
    /// The donor already wrote past the requested midpoint.
    DonorAlreadyPast,
    /// The midpoint leaves an empty half.
    MidpointNotSplittable,
}

/// A live segment as seen by the donor selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DonorView {
    /// Segment identifier.
    pub id: u32,
    /// First byte of the segment.
    pub start: u64,
    /// Last byte of the segment.
    pub end: u64,
    /// Next byte the worker will write.
    pub next: u64,
}

/// The donor chosen for a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DonorChoice {
    /// Segment identifier of the donor.
    pub id: u32,
    /// Midpoint the donor must clamp at.
    pub midpoint: u64,
    /// First byte the new worker will request.
    pub clamped_next: u64,
}

/// Bytes still to download in `[next, end]`.
pub fn remaining(end: u64, next: u64) -> u64 {
    if next > end {
        0
    } else {
        end - next + 1
    }
}

/// Connection count implied by the file size and the user's preference (1-32).
pub fn connections_for(total: u64, requested: u32) -> u32 {
    let requested = requested.clamp(1, 32);
    let by_size = if total < 5 * MB {
        1
    } else if total < 50 * MB {
        4
    } else {
        requested
    };
    by_size.clamp(1, 32)
}

/// Split `total` bytes into `segments` contiguous ranges.
pub fn segments_for(total: u64, segments: u32) -> Vec<SegmentRange> {
    if total == 0 {
        return Vec::new();
    }
    // Never create more segments than bytes: every range must hold at least one byte.
    let count = u64::from(segments.clamp(1, 32)).min(total) as u32;
    let chunk = total / u64::from(count);
    (0..count)
        .map(|i| {
            let start = u64::from(i) * chunk;
            let end = if i + 1 == count {
                total - 1
            } else {
                u64::from(i + 1) * chunk - 1
            };
            SegmentRange { start, end }
        })
        .collect()
}

/// Initial plan for a freshly probed file.
pub fn initial_segments(total: u64, requested: u32) -> Vec<SegmentRange> {
    segments_for(total, connections_for(total, requested))
}

/// How many bytes a worker may read right now without passing its segment end.
pub fn clamp_read(next: u64, end: u64, buf_len: usize) -> usize {
    if next > end {
        return 0;
    }
    let left = end - next + 1;
    usize::try_from(left.min(buf_len as u64)).unwrap_or(buf_len)
}

/// Midpoint of the region still owned by a donor.
pub fn midpoint_of(end: u64, next: u64) -> u64 {
    next.saturating_add(remaining(end, next) / 2)
}

/// Apply the handoff arithmetic. The donor keeps `[start, midpoint]` and the new
/// worker takes `[midpoint + 1, end]`: no gap, no overlap, no byte written twice.
pub fn split_at(donor: SegmentRange, next: u64, midpoint: u64) -> SplitOutcome {
    if next > midpoint {
        return SplitOutcome::Rejected(SplitReject::DonorAlreadyPast);
    }
    if midpoint >= donor.end || midpoint < next {
        return SplitOutcome::Rejected(SplitReject::MidpointNotSplittable);
    }
    SplitOutcome::Accepted {
        donor: SegmentRange { start: donor.start, end: midpoint },
        new_segment: SegmentRange { start: midpoint + 1, end: donor.end },
        clamped_next: midpoint + 1,
    }
}

/// Pick the donor with the most bytes left, if any is worth splitting.
pub fn choose_donor(candidates: &[DonorView]) -> Option<DonorChoice> {
    let best = candidates
        .iter()
        .map(|view| (view, remaining(view.end, view.next)))
        .filter(|(_, left)| *left >= MIN_SPLIT_SIZE)
        .max_by_key(|(_, left)| *left)?;
    let (view, _) = best;
    let midpoint = midpoint_of(view.end, view.next);
    Some(DonorChoice {
        id: view.id,
        midpoint,
        clamped_next: midpoint + 1,
    })
}
