//! `segmenta` command line surface: clap parsing, dispatch and exit codes.
//!
//! Exit codes: `0` success, `1` daemon/protocol failure, `2` invalid usage (clap).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use crate::client::{Client, ClientError};
use crate::render;
use serde_json::{Value, json};

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

/// Parse the arguments and run one command.
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
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

fn run(cli: Cli) -> Result<String, ClientError> {
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(crate::client::default_socket_path);
    let mut client = Client::connect(&socket)?;
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
                        return Err(ClientError::Protocol(format!(
                            "header `{header}` must be \"Name: value\""
                        )));
                    };
                    map.insert(key.trim().to_string(), Value::String(value.trim().to_string()));
                }
                params["headers"] = Value::Object(map);
            }
            let result = client.call("download.add", params)?;
            if json_out {
                out.push_str(&format!("{result}\n"));
            } else {
                out.push_str(&format!(
                    "added {}\n  file: {}\n  status: {}\n",
                    result["task_id"].as_str().unwrap_or("-"),
                    result["filename"].as_str().unwrap_or("-"),
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
                let views: Vec<segmenta_daemon::rpc::DownloadView> =
                    serde_json::from_value(result).map_err(|err| {
                        ClientError::Protocol(format!("unexpected download.list payload: {err}"))
                    })?;
                out.push_str(&render::render_table(&views));
            }
        }
        Command::Status { task_id } => {
            let result = client.call("download.status", json!({ "task_id": task_id }))?;
            if json_out {
                out.push_str(&format!("{result}\n"));
            } else {
                let view: segmenta_daemon::rpc::DownloadView =
                    serde_json::from_value(result).map_err(|err| {
                        ClientError::Protocol(format!("unexpected download.status payload: {err}"))
                    })?;
                out.push_str(&render::render_status(&view));
            }
        }
        Command::Pause { task_id } => {
            let result = client.call("download.pause", json!({ "task_id": task_id }))?;
            out.push_str(&format!("paused {}\n", result["task_id"].as_str().unwrap_or("-")));
        }
        Command::Resume { task_id } => {
            let result = client.call("download.resume", json!({ "task_id": task_id }))?;
            out.push_str(&format!(
                "resumed {} ({})\n",
                result["task_id"].as_str().unwrap_or("-"),
                result["status"].as_str().unwrap_or("-")
            ));
        }
        Command::Cancel {
            task_id,
            delete_file,
        } => {
            let result = client.call(
                "download.cancel",
                json!({ "task_id": task_id, "delete_file": delete_file }),
            )?;
            out.push_str(&format!(
                "cancelled {} (file deleted: {})\n",
                result["task_id"].as_str().unwrap_or("-"),
                result["file_deleted"].as_bool().unwrap_or(false)
            ));
        }
        Command::Delete {
            task_id,
            delete_file,
        } => {
            let result = client.call(
                "download.delete",
                json!({ "task_id": task_id, "delete_file": delete_file }),
            )?;
            out.push_str(&format!(
                "deleted {} (removed: {})\n",
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
                if pairs.is_empty() {
                    return Err(ClientError::Protocol(
                        "settings set needs at least one KEY VALUE pair".to_string(),
                    ));
                }
                if pairs.len() % 2 != 0 {
                    return Err(ClientError::Protocol(
                        "settings set expects KEY VALUE pairs".to_string(),
                    ));
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
                    out.push_str(&render::render_queues(&result));
                }
            }
            QueueAction::Create { name, max_active } => {
                let result = client.call("queue.create", json!({ "name": name, "max_active": max_active }))?;
                out.push_str(&format!(
                    "created queue {} (max_active {})\n",
                    result["name"].as_str().unwrap_or("-"),
                    result["max_active"].as_u64().unwrap_or(0)
                ));
            }
            QueueAction::Remove { name } => {
                client.call("queue.remove", json!({ "name": name }))?;
                out.push_str(&format!("removed queue {name}\n"));
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
                    "segmenta daemon {} (protocol v{})\n",
                    result["daemon_version"].as_str().unwrap_or("-"),
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
                        "daemon:      {}\nuptime:      {} s\nmemory:      {}\nconnections: {}\nsecrets:     {}\n",
                        result["daemon_version"].as_str().unwrap_or("-"),
                        result["uptime_seconds"].as_u64().unwrap_or(0),
                        render::format_bytes(result["memory_bytes"].as_u64().unwrap_or(0)),
                        result["active_connections"].as_u64().unwrap_or(0),
                        result["secret_backend"].as_str().unwrap_or("-"),
                    ));
                }
            }
            SystemAction::Shutdown => {
                client.call("system.shutdown", json!({}))?;
                out.push_str("daemon stopped\n");
            }
        },
    }
    Ok(out)
}
