//! Catalog parity and locale detection. Spec S21.

use std::collections::HashSet;

use segmenta_i18n::{detect_from, tr, tr_error, ErrorText, Locale, Msg};

#[test]
fn locale_tags_are_normalized() {
    for (tag, expected) in [
        ("es_ES.UTF-8", Locale::Es),
        ("es_AR", Locale::Es),
        ("es", Locale::Es),
        ("pt_BR.UTF-8", Locale::PtBr),
        ("pt_BR", Locale::PtBr),
        ("pt-PT", Locale::PtBr),
        ("en_US.UTF-8", Locale::En),
        ("C", Locale::En),
        ("POSIX", Locale::En),
        ("", Locale::En),
        ("fr_FR.UTF-8", Locale::En),
        ("de", Locale::En),
    ] {
        assert_eq!(Locale::from_tag(tag), expected, "tag `{tag}`");
    }
}

#[test]
fn detection_follows_lc_all_then_lc_messages_then_lang() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    };
    // LC_ALL wins.
    assert_eq!(
        detect_from(env(&[
            ("LC_ALL", "pt_BR.UTF-8"),
            ("LC_MESSAGES", "es_ES"),
            ("LANG", "en_US")
        ])),
        Locale::PtBr
    );
    // C/POSIX is ignored, not treated as a language.
    assert_eq!(
        detect_from(env(&[("LC_ALL", "C"), ("LC_MESSAGES", "es_ES")])),
        Locale::Es
    );
    // LANG alone works.
    assert_eq!(detect_from(env(&[("LANG", "es_ES.UTF-8")])), Locale::Es);
    // Nothing set: English.
    assert_eq!(detect_from(env(&[])), Locale::En);
    // An explicit unknown language stays English instead of falling into es.
    assert_eq!(detect_from(env(&[("LANG", "fr_FR.UTF-8")])), Locale::En);
}

#[test]
fn every_message_exists_in_the_three_catalogs() {
    assert!(Msg::ALL.len() >= 90, "the catalog looks truncated");
    let mut seen = HashSet::new();
    for msg in Msg::ALL {
        assert!(seen.insert(*msg), "duplicate {msg:?}");
        for locale in [Locale::En, Locale::Es, Locale::PtBr] {
            let text = tr(locale, *msg);
            assert!(!text.trim().is_empty(), "{msg:?} is empty in {locale:?}");
            assert!(
                !text.starts_with("Msg::"),
                "{msg:?} leaked a variant name in {locale:?}"
            );
        }
    }
    // The three catalogs must actually differ for the core UI strings: an English
    // fallback copy-pasted into es/pt-BR would pass the checks above but fail here.
    let localized = [
        Msg::WindowTitle,
        Msg::Loading,
        Msg::EmptyTitle,
        Msg::ErrorTitle,
        Msg::Preferences,
        Msg::Pause,
        Msg::Search,
        Msg::Retry,
    ];
    for msg in localized {
        assert_ne!(tr(Locale::En, msg), tr(Locale::Es, msg), "{msg:?} es");
    }
    // Portuguese shares some words with English ("Downloads"); require that the
    // catalog as a whole is translated instead of copied.
    let differing = localized
        .iter()
        .filter(|msg| tr(Locale::En, **msg) != tr(Locale::PtBr, **msg))
        .count();
    assert!(differing >= 4, "pt-BR catalog looks copied from English");
}

#[test]
fn every_error_code_has_a_translation_and_unknown_codes_fall_through() {
    assert_eq!(ErrorText::ALL.len(), 24);
    for error in ErrorText::ALL {
        for locale in [Locale::En, Locale::Es, Locale::PtBr] {
            assert!(
                !tr_error(locale, *error).trim().is_empty(),
                "{error:?} is empty in {locale:?}"
            );
        }
    }
    // The stable codes of OpenSpec 07 plus the engine table map to a message.
    for code in [
        1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1021, 1022, 1023, 1024,
        1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, -32603, -32700,
    ] {
        assert!(ErrorText::from_code(code).is_some(), "code {code}");
    }
    assert_eq!(ErrorText::from_code(4242), None);
    assert_eq!(
        ErrorText::from_code(1007),
        Some(ErrorText::BlockedPrivateIp)
    );
}
