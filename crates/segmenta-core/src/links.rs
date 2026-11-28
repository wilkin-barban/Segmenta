//! Link lists: a pasted block of text becomes a plan the daemon will queue.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T8).
//!
//! # Why the daemon owns this
//!
//! The pattern rules already live in [`crate::batch`], and the scheme rules are the same
//! ones the engine accepts. Duplicating them in the GUI would mean two answers to "what will
//! be downloaded", so the GUI asks the daemon and renders what it gets.
//!
//! # Why rejections are part of the answer
//!
//! Pasting fifty lines and silently queueing forty is worse than queueing forty and saying
//! which ten were refused and why.

use serde::{Deserialize, Serialize};

use crate::batch::{expand, BatchError};

/// Most links one paste may queue.
pub const MAX_LINKS: usize = 500;

/// Why a line was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rejection {
    /// The line is not a URL this application can download.
    NotALink,
    /// A sequential pattern would produce more links than allowed.
    PatternTooMany {
        /// How many the pattern implied.
        wanted: usize,
    },
    /// The same link already appears earlier in the paste.
    Duplicate,
    /// The plan is full; this line and the rest are dropped.
    OverLimit,
}

impl Rejection {
    /// Sentence shown next to the line.
    pub fn reason(&self) -> String {
        match self {
            Rejection::NotALink => "not a downloadable link".to_string(),
            Rejection::PatternTooMany { wanted } => {
                format!("the pattern asks for {wanted} links, which is too many")
            }
            Rejection::Duplicate => "already in the list".to_string(),
            Rejection::OverLimit => format!("the list already holds {MAX_LINKS} links"),
        }
    }
}

/// One refused line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    /// The line as pasted.
    pub line: String,
    /// Why it was refused.
    pub rejection: Rejection,
}

/// What a paste produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrabberPlan {
    /// Links to queue, in paste order, without repetitions.
    pub accepted: Vec<String>,
    /// Lines that were refused, with the reason.
    pub refused: Vec<Refused>,
}

impl GrabberPlan {
    /// One-line summary for the dialog.
    pub fn summary(&self) -> String {
        match (self.accepted.len(), self.refused.len()) {
            (0, 0) => "nothing pasted yet".to_string(),
            (accepted, 0) => format!("{accepted} links ready"),
            (0, refused) => format!("no usable links; {refused} refused"),
            (accepted, refused) => format!("{accepted} links ready, {refused} refused"),
        }
    }
}

/// Turn pasted text into a plan.
///
/// Accepts `http`, `https`, `ftp`, `ftps` and `magnet` exactly as written, expands the
/// sequential patterns `batch` understands, and refuses everything else with a reason
/// instead of dropping it.
pub fn plan(text: &str) -> GrabberPlan {
    let mut accepted: Vec<String> = Vec::new();
    let mut refused: Vec<Refused> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if accepted.len() >= MAX_LINKS {
            refused.push(Refused {
                line: line.to_string(),
                rejection: Rejection::OverLimit,
            });
            continue;
        }
        match expand(line) {
            Err(BatchError::TooMany { wanted, .. }) => refused.push(Refused {
                line: line.to_string(),
                rejection: Rejection::PatternTooMany { wanted },
            }),
            Err(_) => refused.push(Refused {
                line: line.to_string(),
                rejection: Rejection::NotALink,
            }),
            Ok(expanded) => {
                for link in expanded {
                    if !is_downloadable(&link) {
                        refused.push(Refused {
                            line: link,
                            rejection: Rejection::NotALink,
                        });
                        continue;
                    }
                    if accepted.iter().any(|seen| seen == &link) {
                        refused.push(Refused {
                            line: link,
                            rejection: Rejection::Duplicate,
                        });
                        continue;
                    }
                    if accepted.len() >= MAX_LINKS {
                        refused.push(Refused {
                            line: link,
                            rejection: Rejection::OverLimit,
                        });
                        continue;
                    }
                    accepted.push(link);
                }
            }
        }
    }
    GrabberPlan { accepted, refused }
}

/// A scheme the downloader handles.
fn is_downloadable(link: &str) -> bool {
    if link.starts_with("magnet:?") {
        return true;
    }
    let Some((scheme, rest)) = link.split_once("://") else {
        return false;
    };
    if !matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ftp" | "ftps"
    ) {
        return false;
    }
    // A scheme with nothing after it is not a link, however well-formed it looks.
    rest.split('/').next().map(str::is_empty) == Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_supported_scheme_is_accepted_and_junk_is_refused_with_a_reason() {
        let pasted = "\
https://example.test/a.bin
ftp://example.test/b.bin
magnet:?xt=urn:btih:0123456789abcdef
not a link
javascript:alert(1)
# a comment line
";
        let plan = plan(pasted);
        assert_eq!(plan.accepted.len(), 3, "{plan:?}");
        assert_eq!(plan.refused.len(), 2, "{plan:?}");
        assert!(plan
            .refused
            .iter()
            .all(|item| item.rejection == Rejection::NotALink));
        // Comments and blank lines are not refusals.
        assert!(plan.refused.iter().all(|item| !item.line.starts_with('#')));
    }

    #[test]
    fn a_pattern_expands_and_duplicates_are_reported_once() {
        let plan = plan("https://example.test/f[1-3].bin\nhttps://example.test/f2.bin\n");
        assert_eq!(plan.accepted.len(), 3);
        assert_eq!(plan.refused.len(), 1);
        assert_eq!(plan.refused[0].rejection, Rejection::Duplicate);
        assert!(plan.summary().contains('3'), "{}", plan.summary());
    }

    #[test]
    fn an_absurd_pattern_is_refused_by_name_not_truncated() {
        let plan = plan("https://example.test/f[1-100000].bin");
        assert!(plan.accepted.is_empty());
        assert!(matches!(
            plan.refused[0].rejection,
            Rejection::PatternTooMany { .. }
        ));
        assert!(plan.refused[0].rejection.reason().contains("100000"));
    }

    #[test]
    fn the_plan_stops_at_the_limit_and_says_so() {
        let mut pasted = String::new();
        for index in 0..(MAX_LINKS + 5) {
            pasted.push_str(&format!("https://example.test/{index}.bin\n"));
        }
        let plan = plan(&pasted);
        assert_eq!(plan.accepted.len(), MAX_LINKS);
        assert_eq!(plan.refused.len(), 5);
        assert!(plan
            .refused
            .iter()
            .all(|item| item.rejection == Rejection::OverLimit));
    }
}
