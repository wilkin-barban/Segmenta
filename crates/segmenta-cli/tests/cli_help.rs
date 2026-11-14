//! `segmenta --help` completeness (spec R3).
//!
//! The expected subcommands and flags are derived from the clap definition itself, never
//! from a hand-written copy, so a new subcommand cannot be forgotten: if the parser accepts
//! it, the rendered help must document it, and the help may not invent anything.
//!
//! Every assertion reads the *rendered text*, so a renderer bug cannot hide behind the
//! helper that produced it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Command as ClapCommand;
use segmenta_cli::cli;
use segmenta_i18n::Locale;

const LOCALES: [Locale; 3] = [Locale::En, Locale::Es, Locale::PtBr];

/// Indent of a command line in the rendered help.
const COMMAND_INDENT: &str = "  ";
/// Indent of a command's description and of its flag list.
const DETAIL_INDENT: &str = "      ";

/// One rendered command: its line, its one-line description and its flag list.
struct Block {
    head: String,
    description: String,
    flags: String,
}

/// Split the rendered help into command blocks.
fn blocks(help: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for line in help.lines() {
        if let Some(head) = command_head(line) {
            blocks.push(Block {
                head,
                description: String::new(),
                flags: String::new(),
            });
            continue;
        }
        let Some(block) = blocks.last_mut() else {
            continue;
        };
        let Some(detail) = line.strip_prefix(DETAIL_INDENT) else {
            continue;
        };
        let detail = detail.trim();
        if block.description.is_empty() {
            block.description = detail.to_string();
        } else {
            block.flags.push(' ');
            block.flags.push_str(detail);
        }
    }
    blocks
}

/// The command line of a block; `None` for the global option rows and the labels.
fn command_head(line: &str) -> Option<String> {
    let rest = line.strip_prefix(COMMAND_INDENT)?;
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('-') {
        return None;
    }
    Some(rest.trim_end().to_string())
}

/// The command path of a block head: the tokens before the first placeholder.
fn head_path(head: &str) -> Vec<String> {
    head.split_whitespace()
        .take_while(|token| !token.starts_with('<') && !token.starts_with('['))
        .map(str::to_string)
        .collect()
}

/// A command registered in the clap definition, with the flag forms it accepts.
struct Registered {
    path: Vec<String>,
    flags: Vec<String>,
}

/// Walk the clap definition and collect every subcommand, recursively.
fn registered(cmd: &ClapCommand, prefix: &[String], out: &mut Vec<Registered>) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        let mut path = prefix.to_vec();
        path.push(sub.get_name().to_string());
        let flags = sub
            .get_arguments()
            .filter(|arg| !arg.is_positional())
            .flat_map(|arg| {
                let mut forms = Vec::new();
                if let Some(short) = arg.get_short() {
                    forms.push(format!("-{short}"));
                }
                if let Some(long) = arg.get_long() {
                    forms.push(format!("--{long}"));
                }
                forms
            })
            .collect();
        out.push(Registered {
            path: path.clone(),
            flags,
        });
        registered(sub, &path, out);
    }
}

fn all_registered() -> Vec<Registered> {
    let mut all = Vec::new();
    registered(&cli::command(), &[], &mut all);
    all
}

/// Every `-s`/`--long` form found in a rendered flag list.
fn flags_in(text: &str) -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    for token in text.split_whitespace() {
        let token = token.trim_start_matches('[').trim_end_matches(']');
        for form in token.split('|') {
            if form.starts_with('-') && form.len() > 1 {
                flags.insert(form.to_string());
            }
        }
    }
    flags
}

fn block_of<'a>(blocks: &'a [Block], path: &[String]) -> Option<&'a Block> {
    blocks.iter().find(|block| head_path(&block.head) == path)
}

fn assert_help_lists_every_registered_subcommand(locale: Locale) {
    let help = cli::help_text(locale);
    let rendered_blocks = blocks(&help);
    let registered = all_registered();
    assert!(
        registered.len() > 1,
        "the clap definition exposes {} subcommands",
        registered.len()
    );

    for command in &registered {
        let path = command.path.join(" ");
        let Some(block) = block_of(&rendered_blocks, &command.path) else {
            panic!("`{path}` is registered in the clap definition but missing from the {locale:?} help:\n{help}");
        };
        assert!(
            !block.description.trim().is_empty(),
            "`{path}` has no one-line description in the {locale:?} help:\n{help}"
        );
        assert!(
            block.description.lines().count() == 1,
            "`{path}` has a multi-line description in the {locale:?} help: {}",
            block.description
        );
    }

    // A block that no registered command claims would document something the parser rejects.
    assert_eq!(
        rendered_blocks.len(),
        registered.len(),
        "the {locale:?} help documents commands the parser does not accept, or lists one twice:\n{help}"
    );

    // Descriptions are localized: a locale that fell back to English is a missing translation.
    if locale != Locale::En {
        let english = blocks(&cli::help_text(Locale::En));
        for command in &registered {
            let path = command.path.join(" ");
            let translated = &block_of(&rendered_blocks, &command.path)
                .expect("checked above")
                .description;
            let english_text = &block_of(&english, &command.path)
                .expect("en help is complete")
                .description;
            assert_ne!(
                translated, english_text,
                "`{path}` is still in English in the {locale:?} help: {translated}"
            );
        }
    }

    // The whole help must actually differ between locales.
    if locale != Locale::En {
        assert_ne!(help, cli::help_text(Locale::En), "{locale:?} help is en");
    }
}

#[test]
fn the_english_help_lists_every_registered_subcommand() {
    assert_help_lists_every_registered_subcommand(Locale::En);
}

#[test]
fn the_spanish_help_lists_every_registered_subcommand() {
    assert_help_lists_every_registered_subcommand(Locale::Es);
}

#[test]
fn the_portuguese_help_lists_every_registered_subcommand() {
    assert_help_lists_every_registered_subcommand(Locale::PtBr);
}

#[test]
fn every_flag_round_trips_between_the_parser_and_the_help() {
    let registered = all_registered();
    for locale in LOCALES {
        let help = cli::help_text(locale);
        let blocks = blocks(&help);
        for command in &registered {
            let path = command.path.join(" ");
            let block = block_of(&blocks, &command.path)
                .unwrap_or_else(|| panic!("`{path}` is missing from the {locale:?} help"));
            let documented = flags_in(&block.flags);

            for form in &command.flags {
                assert!(
                    documented.contains(form),
                    "`{path}`: the parser accepts {form} but the {locale:?} help does not list it: {help}"
                );
            }
            for form in &documented {
                assert!(
                    command.flags.contains(form),
                    "`{path}`: the {locale:?} help documents {form} but the parser rejects it: {help}"
                );
            }
        }
    }
}

#[test]
fn the_extension_command_and_its_four_actions_are_documented() {
    for locale in LOCALES {
        let help = cli::help_text(locale);
        let blocks = blocks(&help);
        let rendered: Vec<String> = blocks
            .iter()
            .map(|block| head_path(&block.head).join(" "))
            .collect();

        assert!(
            rendered.iter().any(|path| path == "extension"),
            "`extension` is missing from the {locale:?} help:\n{help}"
        );
        for action in ["status", "prepare", "install-native", "open"] {
            let path = format!("extension {action}");
            assert!(
                rendered.contains(&path),
                "`{path}` is missing from the {locale:?} help:\n{help}"
            );
        }

        // The two flags of the `extension` actions must be documented, not just the actions.
        for flag in ["--family", "--browser"] {
            assert!(
                help.contains(flag),
                "`{flag}` is missing from the {locale:?} help:\n{help}"
            );
        }
    }
}

fn binary(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let dir = exe
        .parent()
        .expect("deps dir")
        .parent()
        .expect("target dir");
    let mut candidates = vec![dir.join(name)];
    if let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR") {
        candidates.push(Path::new(&manifest).join("../../target/debug").join(name));
    }
    for candidate in &candidates {
        if candidate.exists() {
            return candidate.clone();
        }
    }
    panic!(
        "{name} is not built at {}; run `cargo build --workspace` or `cargo test --workspace`",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" or ")
    );
}

fn run_help(args: &[&str], tag: &str) -> String {
    let output = Command::new(binary("segmenta-cli"))
        .env("LC_ALL", tag)
        .env("LC_MESSAGES", tag)
        .env("LANG", tag)
        .args(args)
        .output()
        .expect("run segmenta-cli");
    assert!(
        output.status.success(),
        "`segmenta {}` failed with {:?}: {}",
        args.join(" "),
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_help_entry_points_are_localized_and_list_the_extension_command() {
    // `--help`, `-h` and clap's `help` subcommand must all agree, in every locale.
    let cases: [(&str, &str, &str); 3] = [
        (
            "en_US.UTF-8",
            "Commands:",
            "Manage browser extensions and manifests.",
        ),
        (
            "es_ES.UTF-8",
            "Comandos:",
            "Gestiona las extensiones del navegador",
        ),
        (
            "pt_BR.UTF-8",
            "Comandos:",
            "Gerencia as extensões do navegador",
        ),
    ];
    let top_level: Vec<String> = cli::command()
        .get_subcommands()
        .filter(|sub| sub.get_name() != "help")
        .map(|sub| sub.get_name().to_string())
        .collect();
    assert!(top_level.iter().any(|name| name == "extension"));

    for (tag, commands_label, extension_description) in cases {
        for args in [
            vec!["--help"],
            vec!["-h"],
            vec!["help"],
            vec!["--json", "help"],
        ] {
            let help = run_help(&args, tag);
            assert!(
                help.contains(commands_label),
                "`segmenta {}` is not localized for {tag}:\n{help}",
                args.join(" ")
            );
            assert!(
                help.contains(extension_description),
                "`segmenta {}` does not describe `extension` for {tag}:\n{help}",
                args.join(" ")
            );
            for name in &top_level {
                assert!(
                    help.lines()
                        .any(|line| line.trim_start().starts_with(name.as_str())),
                    "`segmenta {}` for {tag} omits `{name}`:\n{help}",
                    args.join(" ")
                );
            }
        }
    }
}
