//! Sequential patterns: `foto_[01-25].jpg` becomes 25 URLs.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T6).
//!
//! # Why a hard ceiling
//!
//! A range is a request to build URLs, and the size is chosen by whoever wrote the pattern.
//! `[1-100000]` would allocate a hundred thousand strings before the user sees anything, so
//! expansion stops at [`MAX_EXPANSION`] and says which range caused it. Refusing loudly is
//! better than either freezing or silently truncating to the first thousand.
//!
//! # What is deliberately not a pattern
//!
//! A `[` without a closing `]`, or a range that is not `N-M`, is left untouched: URLs carry
//! brackets legitimately, and guessing would change links nobody asked to change.

/// Most URLs one pattern may produce.
pub const MAX_EXPANSION: usize = 1000;

/// Why a pattern could not be expanded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BatchError {
    /// The range would produce more URLs than allowed.
    #[error("the pattern asks for {wanted} URLs; the limit is {limit}")]
    TooMany {
        /// How many the pattern implies.
        wanted: usize,
        /// The limit that stopped it.
        limit: usize,
    },
    /// The range ends before it starts.
    #[error("the range `{range}` ends before it starts")]
    Backwards {
        /// The offending range as written.
        range: String,
    },
    /// A bracket group is not a range this build understands.
    #[error("the range `{range}` is not a start-end pair")]
    Malformed {
        /// The offending range as written.
        range: String,
    },
}

/// One bracket group: either characters or numbers, inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Group {
    /// `[a-z]`, single characters.
    Chars(Vec<char>),
    /// `[01-25]`, numbers with the padding of the written bounds.
    Numbers {
        /// Values, in order.
        values: Vec<u64>,
        /// Digits to pad with, taken from the start bound.
        width: usize,
    },
}

impl Group {
    /// Render one member of the group.
    fn render(&self, index: usize) -> String {
        match self {
            Group::Chars(values) => values[index].to_string(),
            Group::Numbers { values, width } => format!("{:0width$}", values[index]),
        }
    }

    fn len(&self) -> usize {
        match self {
            Group::Chars(values) => values.len(),
            Group::Numbers { values, .. } => values.len(),
        }
    }
}

/// Expand a pattern into the URLs it names.
pub fn expand(pattern: &str) -> Result<Vec<String>, BatchError> {
    let segments = split_segments(pattern)?;
    let total: usize = segments
        .iter()
        .map(|segment| match segment {
            Segment::Literal(_) => 1,
            Segment::Group(group) => group.len(),
        })
        .product::<usize>()
        .max(1);
    if total > MAX_EXPANSION {
        return Err(BatchError::TooMany {
            wanted: total,
            limit: MAX_EXPANSION,
        });
    }
    let mut urls = Vec::with_capacity(total);
    for position in 0..total {
        let mut url = String::new();
        for (index, segment) in segments.iter().enumerate() {
            match segment {
                Segment::Literal(text) => url.push_str(text),
                Segment::Group(group) => {
                    // Positions are mixed-radix: the last group varies fastest.
                    let factor: usize = segments[index + 1..]
                        .iter()
                        .map(|later| match later {
                            Segment::Literal(_) => 1,
                            Segment::Group(group) => group.len(),
                        })
                        .product();
                    let member = (position / factor.max(1)) % group.len();
                    url.push_str(&group.render(member));
                }
            }
        }
        urls.push(url);
    }
    Ok(urls)
}

/// A pattern split into fixed text and ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Group(Group),
}

fn split_segments(pattern: &str) -> Result<Vec<Segment>, BatchError> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|offset| open + offset) else {
            break;
        };
        let body = &rest[open + 1..close];
        match parse_group(body)? {
            Some(group) => {
                literal.push_str(&rest[..open]);
                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }
                segments.push(Segment::Group(group));
            }
            None => literal.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    if segments.is_empty() {
        segments.push(Segment::Literal(pattern.to_string()));
    }
    Ok(segments)
}

/// Parse `1-3`, `01-25` or `a-z`; `None` when the group is not a range.
fn parse_group(body: &str) -> Result<Option<Group>, BatchError> {
    let malformed = || BatchError::Malformed {
        range: body.to_string(),
    };
    if body.is_empty() {
        return Ok(None);
    }
    if let Some((start, end)) = body.split_once('-') {
        let digits = !start.is_empty() && start.chars().all(|c| c.is_ascii_digit());
        if digits {
            if end.is_empty() || !end.chars().all(|c| c.is_ascii_digit()) {
                return Err(malformed());
            }
            let from: u64 = start.parse().map_err(|_| malformed())?;
            let to: u64 = end.parse().map_err(|_| malformed())?;
            if to < from {
                return Err(BatchError::Backwards {
                    range: body.to_string(),
                });
            }
            let width = start.len();
            let values: Vec<u64> = (from..=to).collect();
            return Ok(Some(Group::Numbers { values, width }));
        }
        let mut chars = start.chars();
        let mut end_chars = end.chars();
        if let (Some(first), None, Some(last), None, true) = (
            chars.next(),
            chars.next(),
            end_chars.next(),
            end_chars.next(),
            start.len() == 1,
        ) {
            if last < first {
                return Err(BatchError::Backwards {
                    range: body.to_string(),
                });
            }
            let values: Vec<char> = (first..=last).collect();
            return Ok(Some(Group::Chars(values)));
        }
    }
    // A single character set such as `[abc]` is not a range: leave it alone.
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_bracket_group_is_literal() {
        assert_eq!(parse_group("abc"), Ok(None));
        assert_eq!(parse_group(""), Ok(None));
        assert!(matches!(
            parse_group("1-"),
            Err(BatchError::Malformed { .. })
        ));
    }
}
