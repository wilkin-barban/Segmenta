//! Pattern expansion for batch capture: `foto_[01-25].jpg` is 25 URLs, not one.

use segmenta_core::batch::{expand, BatchError, MAX_EXPANSION};

#[test]
fn a_numeric_range_expands_with_its_padding() {
    let urls = expand("https://example.test/foto_[01-03].jpg").unwrap();
    assert_eq!(
        urls,
        vec![
            "https://example.test/foto_01.jpg",
            "https://example.test/foto_02.jpg",
            "https://example.test/foto_03.jpg",
        ]
    );
    // No padding in the pattern means no padding in the result.
    let plain = expand("https://example.test/file[8-10].bin").unwrap();
    assert_eq!(plain[0], "https://example.test/file8.bin");
    assert_eq!(plain[2], "https://example.test/file10.bin");
}

#[test]
fn several_groups_multiply_in_order() {
    let urls = expand("https://example.test/[a-b]/[1-2].txt").unwrap();
    assert_eq!(
        urls,
        vec![
            "https://example.test/a/1.txt",
            "https://example.test/a/2.txt",
            "https://example.test/b/1.txt",
            "https://example.test/b/2.txt",
        ]
    );
}

#[test]
fn an_absurd_range_is_refused_instead_of_materialized() {
    let refused = expand("https://example.test/f[1-100000].bin").unwrap_err();
    assert!(matches!(refused, BatchError::TooMany { .. }), "{refused:?}");
    let message = refused.to_string();
    assert!(message.contains("100000"), "{message}");
    assert_eq!(MAX_EXPANSION, 1000);
}

#[test]
fn a_pattern_without_ranges_is_one_url_and_a_backwards_range_is_an_error() {
    assert_eq!(
        expand("https://example.test/plain.bin").unwrap(),
        vec!["https://example.test/plain.bin"]
    );
    assert!(matches!(
        expand("https://example.test/f[9-2].bin").unwrap_err(),
        BatchError::Backwards { .. }
    ));
    assert!(matches!(
        expand("https://example.test/f[1-].bin").unwrap_err(),
        BatchError::Malformed { .. }
    ));
}

#[test]
fn an_unclosed_group_is_left_alone_rather_than_guessed() {
    // A URL with a literal `[` that is not a range must survive untouched: guessing would
    // break any link whose path carries brackets.
    let urls = expand("https://example.test/a[b.bin").unwrap();
    assert_eq!(urls, vec!["https://example.test/a[b.bin"]);
}
