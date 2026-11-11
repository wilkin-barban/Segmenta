//! `segmenta` command line surface: clap parsing, dispatch and exit codes.
//!
//! Exit codes: `0` success, `1` daemon/protocol failure, `2` invalid usage (clap).
//!
//! Every human word printed by the CLI comes from the [`segmenta_i18n`] catalog; the
//! locale is resolved once in [`main`] and threaded down explicitly. Paths, commands,
//! codes, method names and clap's own `--help` text are never translated.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::client::{Client, ClientError};
use crate::render;
use clap::{Parser, Subcommand};
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
        #[arg(short, long, default_value = "all")]
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
    /// Ask the daemon for its protocol and version.
    Hello,
    /// Daemon diagnostics and lifecycle.
    System {
        #[command(subcommand)]
        action: SystemAction,
    },
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


/// Localized `--help` (clap's own help is English-only).
fn help_text(locale: segmenta_i18n::Locale) -> String {
    match locale {
        segmenta_i18n::Locale::En => format!(
            "segmenta {version}\n\nUsage: segmenta [--socket PATH] [--json] <command> [options]\n\n\
             Commands:\n  add <url> [--dir DIR] [--name NAME] [-c N] [-H \"Name: value\"] [--sha256 HEX] [-q QUEUE] [-p N] [--no-start]\n\
             \x20 list [-f all|active|paused|completed|failed] [-l N] [-o N]\n  status <id>\n  pause <id>\n  resume <id>\n\
             \x20 cancel <id> [--delete-file]\n  delete <id> [--delete-file]\n  settings [get|set KEY VALUE ...]\n\
             \x20 queues list|create NAME [--max-active N]|remove NAME|schedule NAME [--start HH:MM] [--stop HH:MM] [--on-complete ACTION]\n\
             \x20 hello\n  system status|shutdown\n\n\
             Global options:\n  --socket PATH   daemon socket (default $SEGMENTA_SOCKET or $XDG_RUNTIME_DIR/segmenta/daemon.sock)\n\
             \x20 --json         machine-readable output\n  --version      print the version\n  --help         this help\n\n\
             Exit codes: 0 success, 1 daemon/protocol error, 2 invalid usage.",
            version = env!("CARGO_PKG_VERSION")
        ),
        segmenta_i18n::Locale::Es => format!(
            "segmenta {version}\n\nUso: segmenta [--socket RUTA] [--json] <comando> [opciones]\n\n\
             Comandos:\n  add <url> [--dir CARPETA] [--name NOMBRE] [-c N] [-H \"Nombre: valor\"] [--sha256 HEX] [-q COLA] [-p N] [--no-start]\n\
             \x20 list [-f all|active|paused|completed|failed] [-l N] [-o N]\n  status <id>\n  pause <id>\n  resume <id>\n\
             \x20 cancel <id> [--delete-file]\n  delete <id> [--delete-file]\n  settings [get|set CLAVE VALOR ...]\n\
             \x20 queues list|create NOMBRE [--max-active N]|remove NOMBRE|schedule NOMBRE [--start HH:MM] [--stop HH:MM] [--on-complete ACCION]\n\
             \x20 hello\n  system status|shutdown\n\n\
             Opciones globales:\n  --socket RUTA   socket del daemon (por defecto $SEGMENTA_SOCKET o $XDG_RUNTIME_DIR/segmenta/daemon.sock)\n\
             \x20 --json         salida legible por máquina\n  --version      muestra la versión\n  --help         esta ayuda\n\n\
             Códigos de salida: 0 éxito, 1 error del daemon o del protocolo, 2 uso inválido.",
            version = env!("CARGO_PKG_VERSION")
        ),
        segmenta_i18n::Locale::PtBr => format!(
            "segmenta {version}\n\nUso: segmenta [--socket CAMINHO] [--json] <comando> [opções]\n\n\
             Comandos:\n  add <url> [--dir PASTA] [--name NOME] [-c N] [-H \"Nome: valor\"] [--sha256 HEX] [-q FILA] [-p N] [--no-start]\n\
             \x20 list [-f all|active|paused|completed|failed] [-l N] [-o N]\n  status <id>\n  pause <id>\n  resume <id>\n\
             \x20 cancel <id> [--delete-file]\n  delete <id> [--delete-file]\n  settings [get|set CHAVE VALOR ...]\n\
             \x20 queues list|create NOME [--max-active N]|remove NOME|schedule NOME [--start HH:MM] [--stop HH:MM] [--on-complete AÇÃO]\n\
             \x20 hello\n  system status|shutdown\n\n\
             Opções globais:\n  --socket CAMINHO   socket do daemon (padrão $SEGMENTA_SOCKET ou $XDG_RUNTIME_DIR/segmenta/daemon.sock)\n\
             \x20 --json         saída legível por máquina\n  --version      mostra a versão\n  --help         esta ajuda\n\n\
             Códigos de saída: 0 sucesso, 1 erro do daemon ou do protocolo, 2 uso inválido.",
            version = env!("CARGO_PKG_VERSION")
        ),
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

/// Parse the arguments and run one command.
pub fn main() -> ExitCode {
    // `--help` must follow LC_ALL/LC_MESSAGES/LANG like every other visible string.
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", help_text(segmenta_i18n::current()));
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
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
