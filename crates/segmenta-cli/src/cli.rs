//! `segmenta` command line surface: clap parsing, dispatch and exit codes.
//!
//! Exit codes: `0` success, `1` daemon/protocol failure, `2` invalid usage (clap).
//!
//! Every human word printed by the CLI comes from the [`segmenta_i18n`] catalog or from
//! the help table below; the locale is resolved once in [`main`] and threaded down
//! explicitly. Paths, commands, codes and method names are never translated.
//!
//! The `--help` text is generated from the clap definition itself (see [`help_text`]), so
//! the documented subcommands and flags cannot drift away from the accepted ones.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::client::{Client, ClientError};
use crate::render;
use clap::{Arg, ArgAction, Command as ClapCommand, CommandFactory, Parser, Subcommand};
use segmenta_i18n::{tr, Locale, Msg};
use serde_json::{json, Value};

#[derive(Parser, Debug)]
#[command(
    name = "segmenta",
    version,
    about = "Control the Segmenta download daemon",
    long_about = "Add, inspect and control downloads on the Segmenta daemon.\n\
                  The daemon must be running (systemctl --user start segmenta.socket)."
)]
struct Cli {
    /// Daemon socket (defaults to $SEGMENTA_SOCKET or $XDG_RUNTIME_DIR/segmenta/daemon.sock).
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Print machine-readable JSON instead of tables.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Add a download.
    Add {
        /// HTTP or HTTPS URL.
        url: String,
        /// Destination directory (absolute).
        #[arg(short, long)]
        dir: Option<String>,
        /// File name to use instead of the server's.
        #[arg(short, long)]
        name: Option<String>,
        /// Connections, 1-32.
        #[arg(short = 'c', long)]
        connections: Option<u32>,
        /// Extra header, "Name: value". Repeatable.
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,
        /// Expected SHA-256 of the finished file.
        #[arg(long)]
        sha256: Option<String>,
        /// Queue name.
        #[arg(short = 'q', long, default_value = "main")]
        queue: String,
        /// Priority, higher runs first.
        #[arg(short = 'p', long, default_value_t = 0)]
        priority: i64,
        /// Register the task without starting it.
        #[arg(long)]
        no_start: bool,
    },
    /// List downloads.
    List {
        /// all, active, paused, completed, failed, cancelled or pending.
        #[arg(
            short,
            long,
            default_value = "all",
            value_name = "all|active|paused|completed|failed|cancelled|pending"
        )]
        filter: String,
        /// Page size.
        #[arg(short, long, default_value_t = 100)]
        limit: usize,
        /// Page offset.
        #[arg(short = 'o', long, default_value_t = 0)]
        offset: usize,
    },
    /// Show one download, including its segments.
    Status { task_id: String },
    /// Pause a download, keeping its bytes for a resume.
    Pause { task_id: String },
    /// Resume a paused download.
    Resume { task_id: String },
    /// Cancel a download.
    Cancel {
        task_id: String,
        /// Also delete the partial file.
        #[arg(long)]
        delete_file: bool,
    },
    /// Remove the task from the daemon (and optionally its file).
    Delete {
        task_id: String,
        /// Also delete the file.
        #[arg(long)]
        delete_file: bool,
    },
    /// Read or update settings.
    Settings {
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },
    /// Manage queues.
    Queues {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// Media helpers for YouTube and public streams.
    Media {
        #[command(subcommand)]
        action: MediaAction,
    },
    /// Torrent helpers for magnet links and .torrent files.
    Torrent {
        #[command(subcommand)]
        action: TorrentAction,
    },
    /// Manage browser extensions and manifests.
    Extension {
        #[command(subcommand)]
        action: ExtensionAction,
    },
    /// Ask the daemon for its protocol and version.
    Hello,
    /// Daemon diagnostics and lifecycle.
    System {
        #[command(subcommand)]
        action: SystemAction,
    },
}

#[derive(Subcommand, Debug)]
enum MediaAction {
    /// Inspect a media URL without downloading it.
    Probe {
        /// https media URL.
        url: String,
        /// video, playlist or stream.
        #[arg(long, default_value = "video")]
        kind: String,
        /// Playlist preview limit.
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Enqueue a media download.
    Add {
        /// https media URL.
        url: String,
        /// video, audio or playlist.
        #[arg(long, default_value = "video", value_name = "video|audio|playlist")]
        mode: String,
        /// Backend format id.
        #[arg(long)]
        format_id: Option<String>,
        /// Requested vertical resolution.
        #[arg(long)]
        height: Option<u32>,
        /// Playlist limit (1-500, default 100).
        #[arg(long)]
        limit: Option<u32>,
    },
}

#[derive(Subcommand, Debug)]
enum ExtensionAction {
    /// Show extension status across browsers.
    Status,
    /// Unzip and prepare browser extension.
    Prepare {
        #[arg(long, default_value = "all")]
        family: String,
    },
    /// Install per-user native messaging manifests.
    InstallNative,
    /// Open browser extensions page.
    Open {
        #[arg(long)]
        browser: String,
    },
}

#[derive(Subcommand, Debug)]
enum TorrentAction {
    /// Inspect a .torrent URL or magnet without downloading it.
    Probe { url: String },
    /// Enqueue a .torrent URL or magnet link.
    Add {
        url: String,
        /// Destination directory (absolute).
        #[arg(long)]
        dir: Option<String>,
        /// Keep seeding after the download completes.
        #[arg(long)]
        seed: bool,
    },
    /// Show one torrent task.
    Status { task_id: String },
}

#[derive(Subcommand, Debug)]
enum SettingsAction {
    /// Print the effective settings.
    Get,
    /// Update settings: `settings set max_connections 8`.
    Set { pairs: Vec<String> },
}

#[derive(Subcommand, Debug)]
enum QueueAction {
    /// List queues with their counters.
    List,
    /// Create a queue.
    Create {
        name: String,
        #[arg(long, default_value_t = 3)]
        max_active: u32,
    },
    /// Remove a queue; its pending tasks move to main.
    Remove { name: String },
    /// Store a schedule for a queue.
    Schedule {
        name: String,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        stop: Option<String>,
        #[arg(long, default_value = "none")]
        on_complete: String,
    },
}

#[derive(Subcommand, Debug)]
enum SystemAction {
    /// Version, uptime, memory and live connections.
    Status,
    /// Pause every task and stop the daemon.
    Shutdown,
}

/// The clap definition of the CLI surface.
///
/// The help renderer and its tests walk this definition, so a subcommand or a flag cannot
/// be added without the documentation and the tests noticing.
pub fn command() -> ClapCommand {
    Cli::command()
}

/// Indent of one command line in the rendered help.
const HELP_COMMAND_INDENT: &str = "  ";
/// Indent of a command's description and of its flag list.
const HELP_DETAIL_INDENT: &str = "      ";
/// Column where flag lists wrap.
const HELP_WIDTH: usize = 100;

/// Every help string that is not derived from the clap definition: the section labels, the
/// global option help and the one-line description of each command.
///
/// The key is a stable command path (`"media.add"`) or a fixed label (`"usage"`), and every
/// entry carries the three locales, so no locale can silently fall back to English.
const HELP_TEXT: &[(&str, &str, &str, &str)] = &[
    (
        "about",
        "Control the Segmenta download daemon.",
        "Controla el daemon de descargas de Segmenta.",
        "Controla o daemon de downloads do Segmenta.",
    ),
    ("usage", "Usage:", "Uso:", "Uso:"),
    ("commands", "Commands:", "Comandos:", "Comandos:"),
    (
        "global-options",
        "Global options:",
        "Opciones globales:",
        "Opções globais:",
    ),
    (
        "exit-codes",
        "Exit codes: 0 success, 1 daemon/protocol error, 2 invalid usage.",
        "Códigos de salida: 0 éxito, 1 error del daemon o del protocolo, 2 uso inválido.",
        "Códigos de saída: 0 sucesso, 1 erro do daemon ou do protocolo, 2 uso inválido.",
    ),
    (
        "opt.socket",
        "daemon socket (default $SEGMENTA_SOCKET or $XDG_RUNTIME_DIR/segmenta/daemon.sock)",
        "socket del daemon (por defecto $SEGMENTA_SOCKET o $XDG_RUNTIME_DIR/segmenta/daemon.sock)",
        "socket do daemon (padrão $SEGMENTA_SOCKET ou $XDG_RUNTIME_DIR/segmenta/daemon.sock)",
    ),
    (
        "opt.json",
        "machine-readable output",
        "salida legible por máquina",
        "saída legível por máquina",
    ),
    ("opt.help", "this help", "esta ayuda", "esta ajuda"),
    (
        "opt.version",
        "print the version",
        "muestra la versión",
        "mostra a versão",
    ),
    (
        "add",
        "Add a download.",
        "Añade una descarga.",
        "Adiciona um download.",
    ),
    (
        "list",
        "List downloads.",
        "Lista las descargas.",
        "Lista os downloads.",
    ),
    (
        "status",
        "Show one download, including its segments.",
        "Muestra una descarga, incluidos sus segmentos.",
        "Mostra um download, incluindo os seus segmentos.",
    ),
    (
        "pause",
        "Pause a download, keeping its bytes for a resume.",
        "Pausa una descarga y conserva sus bytes para reanudarla.",
        "Pausa um download e conserva os bytes para retomá-lo.",
    ),
    (
        "resume",
        "Resume a paused download.",
        "Reanuda una descarga en pausa.",
        "Retoma um download pausado.",
    ),
    (
        "cancel",
        "Cancel a download.",
        "Cancela una descarga.",
        "Cancela um download.",
    ),
    (
        "delete",
        "Remove the task from the daemon (and optionally its file).",
        "Elimina la tarea del daemon (y opcionalmente su archivo).",
        "Remove a tarefa do daemon (e opcionalmente o seu arquivo).",
    ),
    (
        "settings",
        "Read or update settings.",
        "Lee o actualiza los ajustes.",
        "Lê ou atualiza as configurações.",
    ),
    (
        "settings.get",
        "Print the effective settings.",
        "Muestra los ajustes efectivos.",
        "Mostra as configurações efetivas.",
    ),
    (
        "settings.set",
        "Update settings: `settings set max_connections 8`.",
        "Actualiza ajustes: `settings set max_connections 8`.",
        "Atualiza configurações: `settings set max_connections 8`.",
    ),
    (
        "queues",
        "Manage queues.",
        "Gestiona las colas.",
        "Gerencia as filas.",
    ),
    (
        "queues.list",
        "List queues with their counters.",
        "Lista las colas con sus contadores.",
        "Lista as filas com os seus contadores.",
    ),
    (
        "queues.create",
        "Create a queue.",
        "Crea una cola.",
        "Cria uma fila.",
    ),
    (
        "queues.remove",
        "Remove a queue; its pending tasks move to main.",
        "Elimina una cola; sus tareas pendientes pasan a main.",
        "Remove uma fila; as suas tarefas pendentes passam para main.",
    ),
    (
        "queues.schedule",
        "Store a schedule for a queue.",
        "Guarda un horario para una cola.",
        "Guarda um horário para uma fila.",
    ),
    (
        "media",
        "Media helpers for YouTube and public streams.",
        "Ayudas multimedia para YouTube y streams públicos.",
        "Auxiliares de mídia para YouTube e streams públicos.",
    ),
    (
        "media.probe",
        "Inspect a media URL without downloading it.",
        "Inspecciona una URL de medios sin descargarla.",
        "Inspeciona uma URL de mídia sem baixá-la.",
    ),
    (
        "media.add",
        "Enqueue a media download.",
        "Encola una descarga multimedia.",
        "Enfileira um download de mídia.",
    ),
    (
        "torrent",
        "Torrent helpers for magnet links and .torrent files.",
        "Ayudas para torrents con enlaces magnet y archivos .torrent.",
        "Auxiliares de torrent para links magnet e arquivos .torrent.",
    ),
    (
        "torrent.probe",
        "Inspect a .torrent URL or magnet without downloading it.",
        "Inspecciona una URL .torrent o un magnet sin descargarlo.",
        "Inspeciona uma URL .torrent ou um magnet sem baixá-lo.",
    ),
    (
        "torrent.add",
        "Enqueue a .torrent URL or magnet link.",
        "Encola una URL .torrent o un enlace magnet.",
        "Enfileira uma URL .torrent ou um link magnet.",
    ),
    (
        "torrent.status",
        "Show one torrent task.",
        "Muestra una tarea de torrent.",
        "Mostra uma tarefa de torrent.",
    ),
    (
        "extension",
        "Manage browser extensions and manifests.",
        "Gestiona las extensiones del navegador y sus manifiestos.",
        "Gerencia as extensões do navegador e seus manifestos.",
    ),
    (
        "extension.status",
        "Show extension status across browsers.",
        "Muestra el estado de la extensión en cada navegador.",
        "Mostra o estado da extensão em cada navegador.",
    ),
    (
        "extension.prepare",
        "Unzip and prepare browser extension.",
        "Descomprime y prepara la extensión del navegador.",
        "Descompacta e prepara a extensão do navegador.",
    ),
    (
        "extension.install-native",
        "Install per-user native messaging manifests.",
        "Instala los manifiestos de mensajería nativa por usuario.",
        "Instala os manifestos de mensagens nativas por usuário.",
    ),
    (
        "extension.open",
        "Open browser extensions page.",
        "Abre la página de extensiones del navegador.",
        "Abre a página de extensões do navegador.",
    ),
    (
        "hello",
        "Ask the daemon for its protocol and version.",
        "Consulta al daemon su protocolo y versión.",
        "Consulta o daemon sobre o seu protocolo e versão.",
    ),
    (
        "system",
        "Daemon diagnostics and lifecycle.",
        "Diagnóstico y ciclo de vida del daemon.",
        "Diagnóstico e ciclo de vida do daemon.",
    ),
    (
        "system.status",
        "Version, uptime, memory and live connections.",
        "Versión, tiempo activo, memoria y conexiones activas.",
        "Versão, tempo ativo, memória e conexões ativas.",
    ),
    (
        "system.shutdown",
        "Pause every task and stop the daemon.",
        "Pausa todas las tareas y detiene el daemon.",
        "Pausa todas as tarefas e para o daemon.",
    ),
];

/// Look up one help string; `None` when the key is not in [`HELP_TEXT`].
fn help_word(locale: Locale, key: &str) -> Option<&'static str> {
    HELP_TEXT
        .iter()
        .find(|(candidate, ..)| *candidate == key)
        .map(|(_, en, es, pt_br)| match locale {
            Locale::En => *en,
            Locale::Es => *es,
            Locale::PtBr => *pt_br,
        })
}

/// A section label. A missing key renders empty instead of panicking inside `--help`.
fn word(locale: Locale, key: &str) -> &'static str {
    help_word(locale, key).unwrap_or_default()
}

/// The localized `--help`, generated from the clap definition.
///
/// Every command line, positional and flag is derived from [`command`], so the documented
/// surface is exactly the accepted one. Only the descriptions and labels come from
/// [`HELP_TEXT`].
pub fn help_text(locale: Locale) -> String {
    let root = command();
    let mut out = format!("segmenta {}\n", env!("CARGO_PKG_VERSION"));
    out.push_str(word(locale, "about"));
    out.push_str("\n\n");

    // Usage line: the global options and the subcommand placeholder come from clap.
    out.push_str(word(locale, "usage"));
    out.push(' ');
    out.push_str(root.get_name());
    for arg in root.get_arguments().filter(|arg| !arg.is_positional()) {
        out.push(' ');
        out.push_str(&optional(arg, &option_usage(arg)));
    }
    out.push(' ');
    out.push_str(if root.is_subcommand_required_set() {
        "<COMMAND>"
    } else {
        "[<COMMAND>]"
    });
    out.push_str("\n\n");

    out.push_str(word(locale, "commands"));
    out.push('\n');
    for sub in visible_subcommands(&root) {
        render_command(&mut out, locale, "", sub);
    }

    out.push('\n');
    out.push_str(word(locale, "global-options"));
    out.push('\n');
    let mut rows: Vec<(String, &'static str)> = root
        .get_arguments()
        .filter(|arg| !arg.is_positional())
        .map(|arg| {
            let key = format!("opt.{}", arg.get_long().unwrap_or_default());
            (option_usage(arg), word(locale, &key))
        })
        .collect();
    rows.push(("-h|--help".to_string(), word(locale, "opt.help")));
    rows.push(("-V|--version".to_string(), word(locale, "opt.version")));
    let width = rows.iter().map(|(flags, _)| flags.len()).max().unwrap_or(0) + 2;
    for (flags, text) in rows {
        out.push_str(&format!("{HELP_COMMAND_INDENT}{flags:<width$}{text}\n"));
    }

    out.push('\n');
    out.push_str(word(locale, "exit-codes"));
    out.push('\n');
    out
}

/// Render one command: its line, its one-line description and its flag list, then its own
/// subcommands.
fn render_command(out: &mut String, locale: Locale, parent: &str, cmd: &ClapCommand) {
    let path = if parent.is_empty() {
        cmd.get_name().to_string()
    } else {
        format!("{parent} {}", cmd.get_name())
    };
    out.push_str(HELP_COMMAND_INDENT);
    out.push_str(&path);
    for arg in cmd.get_positionals() {
        out.push(' ');
        out.push_str(&positional_usage(arg));
    }
    if has_visible_subcommands(cmd) {
        out.push(' ');
        out.push_str(if cmd.is_subcommand_required_set() {
            "<COMMAND>"
        } else {
            "[<COMMAND>]"
        });
    }
    out.push('\n');

    out.push_str(HELP_DETAIL_INDENT);
    out.push_str(&command_description(locale, cmd, &path));
    out.push('\n');

    let flags: Vec<String> = cmd
        .get_arguments()
        .filter(|arg| !arg.is_positional())
        .map(|arg| optional(arg, &option_usage(arg)))
        .collect();
    push_wrapped(out, HELP_DETAIL_INDENT, &flags);

    for sub in visible_subcommands(cmd) {
        render_command(out, locale, &path, sub);
    }
}

/// The localized one-line description of a command, falling back to the clap doc comment.
fn command_description(locale: Locale, cmd: &ClapCommand, path: &str) -> String {
    let key = command_key(path);
    if let Some(description) = help_word(locale, &key) {
        return description.to_string();
    }
    cmd.get_about()
        .map(|about| about.to_string())
        .unwrap_or_default()
}

/// The [`HELP_TEXT`] key of a rendered command path (`"media add"` -> `"media.add"`).
fn command_key(path: &str) -> String {
    path.split_whitespace().collect::<Vec<_>>().join(".")
}

/// Subcommands as a user sees them (`help` is clap's own alias and is handled by [`main`]).
fn visible_subcommands(cmd: &ClapCommand) -> impl Iterator<Item = &ClapCommand> {
    cmd.get_subcommands().filter(|sub| sub.get_name() != "help")
}

fn has_visible_subcommands(cmd: &ClapCommand) -> bool {
    visible_subcommands(cmd).next().is_some()
}

/// `[--dir DIR]` for an optional option, `--browser BROWSER` for a required one.
fn optional(arg: &Arg, usage: &str) -> String {
    if arg.is_required_set() {
        usage.to_string()
    } else {
        format!("[{usage}]")
    }
}

/// `--long VALUE`, `-s|--long VALUE` or a bare flag, in the order clap stores the argument.
fn option_usage(arg: &Arg) -> String {
    let mut usage = match (arg.get_short(), arg.get_long()) {
        (Some(short), Some(long)) => format!("-{short}|--{long}"),
        (Some(short), None) => format!("-{short}"),
        (None, Some(long)) => format!("--{long}"),
        (None, None) => String::new(),
    };
    if takes_value(arg) {
        usage.push(' ');
        usage.push_str(&value_name(arg));
    }
    usage
}

/// `<URL>`, `<URL>...` or `[PAIRS]...` for a positional.
fn positional_usage(arg: &Arg) -> String {
    let name = value_name(arg);
    let mut usage = if arg.is_required_set() {
        format!("<{name}>")
    } else {
        format!("[{name}]")
    };
    if is_multiple(arg) {
        usage.push_str("...");
    }
    usage
}

/// The placeholder clap would use: an explicit `value_name`, else the long name, else the id.
fn value_name(arg: &Arg) -> String {
    if let Some(name) = arg.get_value_names().and_then(|names| names.first()) {
        return name.to_string();
    }
    if let Some(long) = arg.get_long() {
        return long.to_uppercase().replace('-', "_");
    }
    arg.get_id().as_str().to_uppercase()
}

/// Whether the argument consumes a value (`--dir DIR`) or is a bare flag (`--no-start`).
fn takes_value(arg: &Arg) -> bool {
    if matches!(
        arg.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Help | ArgAction::Version
    ) {
        return false;
    }
    arg.get_num_args()
        .map(|range| range.max_values() > 0)
        .unwrap_or(true)
}

/// Whether the argument accepts several values (`[PAIRS]...`).
fn is_multiple(arg: &Arg) -> bool {
    matches!(arg.get_action(), ArgAction::Append | ArgAction::Count)
        || arg
            .get_num_args()
            .map(|range| range.max_values() > 1)
            .unwrap_or(false)
}

/// Append `items` as space-separated lines of at most [`HELP_WIDTH`] columns.
fn push_wrapped(out: &mut String, indent: &str, items: &[String]) {
    let mut line = String::new();
    for item in items {
        if !line.is_empty() && indent.len() + line.len() + 1 + item.len() > HELP_WIDTH {
            out.push_str(indent);
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(item);
    }
    if !line.is_empty() {
        out.push_str(indent);
        out.push_str(&line);
        out.push('\n');
    }
}

/// Resolve a task id: full UUIDs pass through, a unique prefix is expanded using
/// `download.list` so the 8-character id printed by `segmenta list` is enough.
fn resolve_task_id(
    client: &mut Client,
    locale: Locale,
    given: &str,
) -> Result<String, ClientError> {
    if given.len() >= 36 {
        return Ok(given.to_string());
    }
    let listed = client.call("download.list", json!({ "filter": "all", "limit": 1000 }))?;
    let ids: Vec<String> = listed
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["task_id"].as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let matches: Vec<&String> = ids.iter().filter(|id| id.starts_with(given)).collect();
    match matches.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(ClientError::Usage(format!(
            "{} `{given}`",
            tr(locale, Msg::CliShortIdUnknown)
        ))),
        _ => Err(ClientError::Usage(format!(
            "`{given}` {}",
            tr(locale, Msg::CliShortIdAmbiguous)
        ))),
    }
}

/// Whether the arguments ask for the localized help: `--help`/`-h` anywhere, or clap's own
/// `help` subcommand in the command position.
fn help_requested(args: &[String]) -> bool {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return true;
    }
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => index += 1,
            "--socket" => index += 2,
            arg if arg.starts_with("--socket=") => index += 1,
            arg => return arg == "help",
        }
    }
    false
}

/// Parse the arguments and run one command.
pub fn main() -> ExitCode {
    // `--help` must follow LC_ALL/LC_MESSAGES/LANG like every other visible string.
    let args: Vec<String> = std::env::args().collect();
    if help_requested(&args) {
        println!("{}", help_text(segmenta_i18n::current()));
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse_from(args);
    let locale = segmenta_i18n::current();
    match run(cli, locale) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("segmenta: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli, locale: Locale) -> Result<String, ClientError> {
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(crate::client::default_socket_path);
    let mut client = Client::connect_with(&socket, locale)?;
    let json_out = cli.json;
    let mut out = String::new();

    match cli.command {
        Command::Add {
            url,
            dir,
            name,
            connections,
            headers,
            sha256,
            queue,
            priority,
            no_start,
        } => {
            let mut params = json!({
                "url": url,
                "queue_name": queue,
                "priority": priority,
                "start_immediately": !no_start,
            });
            if let Some(dir) = dir {
                params["destination_dir"] = Value::String(dir);
            }
            if let Some(name) = name {
                params["filename"] = Value::String(name);
            }
            if let Some(connections) = connections {
                params["connections"] = Value::from(connections);
            }
            if let Some(hash) = sha256 {
                params["expected_checksum"] = json!({ "algorithm": "sha256", "hash": hash });
            }
            if !headers.is_empty() {
                let mut map = serde_json::Map::new();
                for header in headers {
                    let Some((key, value)) = header.split_once(':') else {
                        return Err(ClientError::Usage(format!(
                            "{}: `{header}`",
                            tr(locale, Msg::CliHeaderUsage)
                        )));
                    };
                    map.insert(
                        key.trim().to_string(),
                        Value::String(value.trim().to_string()),
                    );
                }
                params["headers"] = Value::Object(map);
            }
            let result = client.call("download.add", params)?;
            if json_out {
                out.push_str(&format!("{result}\n"));
            } else {
                out.push_str(&format!(
                    "{} {}\n  {}: {}\n  {}: {}\n",
                    tr(locale, Msg::CliAdded),
                    result["task_id"].as_str().unwrap_or("-"),
                    tr(locale, Msg::CliFile),
                    result["filename"].as_str().unwrap_or("-"),
                    tr(locale, Msg::CliStatus),
                    result["status"].as_str().unwrap_or("pending"),
                ));
            }
        }
        Command::List {
            filter,
            limit,
            offset,
        } => {
            let result = client.call(
                "download.list",
                json!({ "filter": filter, "limit": limit, "offset": offset }),
            )?;
            if json_out {
                out.push_str(&format!("{result}\n"));
            } else {
                let views: Vec<segmenta_daemon::rpc::DownloadView> = serde_json::from_value(result)
                    .map_err(|err| {
                        ClientError::Protocol(
                            tr(locale, Msg::InvalidDaemonResponse)
                                .replace("{error}", &err.to_string()),
                        )
                    })?;
                out.push_str(&render::render_table(locale, &views));
            }
        }
        Command::Status { task_id } => {
            let task_id = resolve_task_id(&mut client, locale, &task_id)?;
            let result = client.call("download.status", json!({ "task_id": task_id }))?;
            if json_out {
                out.push_str(&format!("{result}\n"));
            } else {
                let view: segmenta_daemon::rpc::DownloadView = serde_json::from_value(result)
                    .map_err(|err| {
                        ClientError::Protocol(
                            tr(locale, Msg::InvalidDaemonResponse)
                                .replace("{error}", &err.to_string()),
                        )
                    })?;
                out.push_str(&render::render_status(locale, &view));
            }
        }
        Command::Pause { task_id } => {
            let task_id = resolve_task_id(&mut client, locale, &task_id)?;
            let result = client.call("download.pause", json!({ "task_id": task_id }))?;
            out.push_str(&format!(
                "{} {}\n",
                tr(locale, Msg::CliPaused),
                result["task_id"].as_str().unwrap_or("-")
            ));
        }
        Command::Resume { task_id } => {
            let task_id = resolve_task_id(&mut client, locale, &task_id)?;
            let result = client.call("download.resume", json!({ "task_id": task_id }))?;
            out.push_str(&format!(
                "{} {} ({})\n",
                tr(locale, Msg::CliResumed),
                result["task_id"].as_str().unwrap_or("-"),
                result["status"].as_str().unwrap_or("-")
            ));
        }
        Command::Cancel {
            task_id,
            delete_file,
        } => {
            let task_id = resolve_task_id(&mut client, locale, &task_id)?;
            let result = client.call(
                "download.cancel",
                json!({ "task_id": task_id, "delete_file": delete_file }),
            )?;
            // `file_deleted` is the protocol field name, kept verbatim on purpose.
            out.push_str(&format!(
                "{} {} (file_deleted: {})\n",
                tr(locale, Msg::CliCancelled),
                result["task_id"].as_str().unwrap_or("-"),
                result["file_deleted"].as_bool().unwrap_or(false)
            ));
        }
        Command::Delete {
            task_id,
            delete_file,
        } => {
            let task_id = resolve_task_id(&mut client, locale, &task_id)?;
            let result = client.call(
                "download.delete",
                json!({ "task_id": task_id, "delete_file": delete_file }),
            )?;
            // `deleted` is the protocol field name, kept verbatim on purpose.
            out.push_str(&format!(
                "{} {} (deleted: {})\n",
                tr(locale, Msg::CliDeleted),
                result["task_id"].as_str().unwrap_or("-"),
                result["deleted"].as_bool().unwrap_or(false)
            ));
        }
        Command::Settings { action } => match action {
            None | Some(SettingsAction::Get) => {
                let result = client.call("settings.get", json!({}))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_settings(&result));
                }
            }
            Some(SettingsAction::Set { pairs }) => {
                if pairs.is_empty() || pairs.len() % 2 != 0 {
                    return Err(ClientError::Usage(format!(
                        "settings set {}",
                        tr(locale, Msg::CliPairUsage)
                    )));
                }
                let mut patch = serde_json::Map::new();
                for pair in pairs.chunks(2) {
                    let key = pair[0].clone();
                    let raw = pair[1].clone();
                    // Accept JSON literals (numbers, booleans, null) and bare strings.
                    let value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
                    patch.insert(key, value);
                }
                let result = client.call("settings.update", Value::Object(patch))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_settings(&result));
                }
            }
        },
        Command::Queues { action } => match action {
            QueueAction::List => {
                let result = client.call("queue.list", json!({}))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_queues(locale, &result));
                }
            }
            QueueAction::Create { name, max_active } => {
                let result = client.call(
                    "queue.create",
                    json!({ "name": name, "max_active": max_active }),
                )?;
                // `max_active` is the protocol field name, kept verbatim on purpose.
                out.push_str(&format!(
                    "{} {} (max_active {})\n",
                    tr(locale, Msg::CliQueueCreated),
                    result["name"].as_str().unwrap_or("-"),
                    result["max_active"].as_u64().unwrap_or(0)
                ));
            }
            QueueAction::Remove { name } => {
                client.call("queue.remove", json!({ "name": name }))?;
                out.push_str(&format!("{} {name}\n", tr(locale, Msg::CliQueueRemoved)));
            }
            QueueAction::Schedule {
                name,
                start,
                stop,
                on_complete,
            } => {
                let result = client.call(
                    "queue.schedule",
                    json!({
                        "queue_name": name,
                        "start_time": start,
                        "stop_time": stop,
                        "action_on_complete": on_complete,
                    }),
                )?;
                out.push_str(&format!("{result}\n"));
            }
        },
        Command::Media { action } => match action {
            MediaAction::Probe { url, kind, limit } => {
                let result = client.call(
                    "media.probe",
                    json!({ "url": url, "kind": kind, "limit": limit }),
                )?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    use segmenta_i18n::{tr, Msg};
                    let locale = segmenta_i18n::current();
                    out.push_str(&format!(
                        "{}: {}\n{}: {}\n{}: {} · {}: {} · {}: {}\n",
                        tr(locale, Msg::MediaTitle),
                        result["title"].as_str().unwrap_or("-"),
                        tr(locale, Msg::MediaKind),
                        result["kind"].as_str().unwrap_or("-"),
                        tr(locale, Msg::MediaFormats),
                        result["formats"].as_array().map(Vec::len).unwrap_or(0),
                        tr(locale, Msg::MediaAudio),
                        result["audio"].as_array().map(Vec::len).unwrap_or(0),
                        tr(locale, Msg::MediaPlaylist),
                        result["playlist"]["count"].as_u64().unwrap_or(0),
                    ));
                }
            }
            MediaAction::Add {
                url,
                mode,
                format_id,
                height,
                limit,
            } => {
                let result = client.call(
                    "media.add",
                    json!({
                        "url": url,
                        "mode": mode,
                        "format_id": format_id,
                        "height": height,
                        "playlist_limit": limit,
                    }),
                )?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    use segmenta_i18n::{tr, Msg};
                    out.push_str(&format!(
                        "{} {} ({})\n",
                        tr(segmenta_i18n::current(), Msg::MediaAdded),
                        result["task_id"].as_str().unwrap_or("-"),
                        result["kind"].as_str().unwrap_or("-"),
                    ));
                }
            }
        },
        Command::Torrent { action } => match action {
            TorrentAction::Probe { url } => {
                let result = client.call("torrent.probe", json!({ "url": url }))?;
                out.push_str(
                    &serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()),
                );
                out.push('\n');
            }
            TorrentAction::Add { url, dir, seed } => {
                let mut params = json!({
                    "url": url,
                    "seed_after_complete": seed,
                });
                if let Some(dir) = dir {
                    params["destination_dir"] = Value::String(dir);
                }
                let result = client.call("torrent.add", params)?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&format!(
                        "{} {} ({})\n",
                        tr(locale, Msg::MediaAdded),
                        result["task_id"].as_str().unwrap_or("-"),
                        result["kind"].as_str().unwrap_or("torrent"),
                    ));
                }
            }
            TorrentAction::Status { task_id } => {
                let result = client.call("torrent.status", json!({ "task_id": task_id }))?;
                out.push_str(
                    &serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()),
                );
                out.push('\n');
            }
        },
        Command::Extension { action } => match action {
            ExtensionAction::Status => {
                let result = client.call("extension.status", json!({}))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_extension_status(locale, &result));
                }
            }
            ExtensionAction::Prepare { family } => {
                let result = client.call("extension.prepare", json!({ "family": family }))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_extension_prepare(locale, &result));
                }
            }
            ExtensionAction::InstallNative => {
                let result = client.call("extension.install-native", json!({}))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_extension_install_native(locale, &result));
                }
            }
            ExtensionAction::Open { browser } => {
                let result = client.call("extension.open", json!({ "browser_id": browser }))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&render::render_extension_open(locale, &result));
                }
            }
        },
        Command::Hello => {
            let result = client.call("system.hello", json!({}))?;
            if json_out {
                out.push_str(&format!("{result}\n"));
            } else {
                out.push_str(&format!(
                    "{} {} ({} v{})\n",
                    tr(locale, Msg::CliHello),
                    result["daemon_version"].as_str().unwrap_or("-"),
                    tr(locale, Msg::CliProtocol),
                    result["protocol_version"].as_u64().unwrap_or(0)
                ));
            }
        }
        Command::System { action } => match action {
            SystemAction::Status => {
                let result = client.call("system.status", json!({}))?;
                if json_out {
                    out.push_str(&format!("{result}\n"));
                } else {
                    out.push_str(&format!(
                        "{:<12} {}\n{:<12} {} s\n{:<12} {}\n{:<12} {}\n{:<12} {}\n",
                        format!("{}:", tr(locale, Msg::DaemonVersion)),
                        result["daemon_version"].as_str().unwrap_or("-"),
                        format!("{}:", tr(locale, Msg::CliUptime)),
                        result["uptime_seconds"].as_u64().unwrap_or(0),
                        format!("{}:", tr(locale, Msg::CliMemory)),
                        render::format_bytes(result["memory_bytes"].as_u64().unwrap_or(0)),
                        format!("{}:", tr(locale, Msg::CliConnections)),
                        result["active_connections"].as_u64().unwrap_or(0),
                        format!("{}:", tr(locale, Msg::CliSecrets)),
                        result["secret_backend"].as_str().unwrap_or("-"),
                    ));
                }
            }
            SystemAction::Shutdown => {
                client.call("system.shutdown", json!({}))?;
                out.push_str(&format!("{}\n", tr(locale, Msg::CliDaemonStopped)));
            }
        },
    }
    Ok(out)
}
