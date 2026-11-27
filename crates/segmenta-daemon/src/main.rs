//! `segmenta-daemon` — download engine, JSON-RPC server and SQLite state.
//!
//! Run by systemd as `segmenta.service` (socket-activated through `segmenta.socket`) or
//! standalone for development. A second instance is rejected through an exclusive
//! `flock` on `$XDG_RUNTIME_DIR/segmenta/daemon.lock`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::{systemd_listen_fd, IpcServer};
use segmenta_daemon::manager::Manager;
use segmenta_daemon::scheduler::{QueueScheduler, SystemActions, TICK_INTERVAL};
use segmenta_daemon::secrets;
use segmenta_daemon::store::Store;
use tokio::sync::watch;

#[derive(Parser, Debug)]
#[command(
    name = "segmenta-daemon",
    version,
    about = "Segmenta download engine daemon"
)]
struct Args {
    /// Adopt the listening socket passed by systemd.
    #[arg(long)]
    systemd: bool,
    /// Socket path override (tests and development).
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Data directory override (database and fallback secrets).
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("segmenta-daemon: cannot start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(args: Args) -> Result<(), String> {
    segmenta_daemon::logging::init();
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(segmenta_daemon::data_dir);
    let runtime_dir = segmenta_daemon::runtime_dir();
    std::fs::create_dir_all(&data_dir)
        .map_err(|err| format!("cannot create {}: {err}", data_dir.display()))?;
    std::fs::create_dir_all(&runtime_dir)
        .map_err(|err| format!("cannot create {}: {err}", runtime_dir.display()))?;
    set_mode(&data_dir, 0o700)?;
    set_mode(&runtime_dir, 0o700)?;

    // Single instance: hold the lock for the whole process lifetime.
    let lock_path = runtime_dir.join("daemon.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|err| format!("cannot open {}: {err}", lock_path.display()))?;
    {
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err(format!(
                "ERR_DAEMON_ALREADY_RUNNING: another segmenta-daemon holds {}",
                lock_path.display()
            ));
        }
    }

    let db_path = data_dir.join("segmenta.db");
    let store =
        Arc::new(Store::open(&db_path).map_err(|err| format!("cannot open the database: {err}"))?);
    let secrets: Arc<dyn segmenta_daemon::secrets::SecretStore> = Arc::from(
        secrets::open_default(&data_dir).map_err(|err| format!("cannot open secrets: {err}"))?,
    );
    let secret_backend = secrets.backend();
    let socket_path = args
        .socket
        .clone()
        .or_else(|| {
            std::env::var_os("SEGMENTA_SOCKET")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(segmenta_daemon::socket_path);
    let bus = Arc::new(EventBus::new());
    let server = match systemd_listen_fd().filter(|_| args.systemd) {
        Some(fd) => IpcServer::from_systemd(fd, &socket_path, Arc::clone(&bus)),
        None => IpcServer::bind(&socket_path, Arc::clone(&bus)).await,
    }
    .map_err(|err| format!("cannot bind {}: {err}", socket_path.display()))?;
    let server = Arc::new(server);

    // The resolver configuration travels with the settings; publishing it once here is
    // enough because `settings.update` republishes it.
    publish_dns(&store);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        secrets,
        Arc::clone(&bus),
        shutdown_tx,
    ));
    manager.recover();
    // The queue scheduler runs next to the rest of the daemon and stops with it: the same
    // shutdown channel that closes the IPC server ends its loop (OpenSpec 12 §2).
    let scheduler =
        QueueScheduler::new(Arc::clone(&manager), Arc::new(SystemActions), TICK_INTERVAL);
    let scheduler = tokio::spawn(scheduler.run(shutdown_rx.clone()));
    // Click'n'Load 2 is opt-in. A port already taken (JDownloader is the usual owner) is a
    // reported condition, not a failure: the feature stays off and nothing retries.
    let cnl2_enabled = store.settings().map(|s| s.cnl2_enabled).unwrap_or(false);
    if cnl2_enabled {
        match segmenta_daemon::cnl2::bind(segmenta_daemon::cnl2::DEFAULT_PORT).await {
            Ok(listener) => {
                tracing::info!(
                    port = segmenta_daemon::cnl2::DEFAULT_PORT,
                    "click'n'load 2 listening"
                );
                tokio::spawn(segmenta_daemon::cnl2::serve(
                    listener,
                    segmenta_daemon::cnl2::DEFAULT_PORT,
                    Arc::clone(&manager),
                    shutdown_rx.clone(),
                ));
            }
            Err(err) => tracing::warn!(
                port = segmenta_daemon::cnl2::DEFAULT_PORT,
                error = %err,
                "click'n'load 2 disabled: the port is not available"
            ),
        }
    }
    // Feeds are polled on their own clock; the loop is cheap and stops with the daemon.
    {
        let manager = Arc::clone(&manager);
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = tick.tick() => {
                        let _ = manager.rss_check_due().await;
                    }
                }
            }
        });
    }
    // With a VPN interface configured, every external tool goes through this relay: it is
    // the only proxy that binds its sockets to the tunnel. Without the interface it never
    // starts, so a user without a VPN keeps the previous behaviour untouched.
    {
        let settings = store.settings().unwrap_or_default();
        let interface = settings.vpn_interface.trim().to_string();
        if !interface.is_empty() {
            match segmenta_daemon::vpn_relay::spawn(
                interface.clone(),
                settings.allow_local_downloads,
            )
            .await
            {
                Ok(relay) => {
                    tracing::info!(
                        port = relay.port,
                        interface = %interface,
                        "vpn relay listening"
                    );
                    segmenta_daemon::vpn_relay::set_active(Some(relay.url.clone()));
                    std::mem::forget(relay);
                }
                Err(err) => tracing::warn!(
                    interface = %interface,
                    error = %err,
                    "the vpn relay could not start; external tools keep their own network"
                ),
            }
        }
    }
    // Remote control is opt-in: the supervisor opens the port only while the setting is on
    // and a token digest exists, and closes it as soon as either goes away.
    tokio::spawn(remote_supervisor(Arc::clone(&manager), shutdown_rx.clone()));
    tracing::info!(
        version = segmenta_daemon::DAEMON_VERSION,
        socket = %socket_path.display(),
        data_dir = %data_dir.display(),
        secret_backend,
        "segmenta-daemon listening"
    );

    // SAFETY: getuid has no preconditions and cannot fail.
    let expected_uid = unsafe { libc::getuid() };
    let handler: Arc<dyn segmenta_daemon::ipc::RpcHandler> = manager.clone();
    let running = Arc::clone(&server);
    let mut serving = tokio::spawn(async move { running.run(handler, expected_uid).await });

    tokio::select! {
        _ = shutdown_rx.changed() => {}
        _ = &mut serving => {}
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate_signal() => {}
    }
    tracing::info!("segmenta-daemon shutting down");
    // Stop the scheduler before the lock is released, so no task starts mid-shutdown.
    manager.request_shutdown();
    let _ = scheduler.await;
    // Keep the lock file alive until the very end.
    drop(lock);
    Ok(())
}

/// Keep the remote API in step with the settings.
async fn remote_supervisor(
    manager: Arc<segmenta_daemon::manager::Manager>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut server: Option<tokio::task::JoinHandle<()>> = None;
    let mut bound: Option<(String, u16)> = None;
    loop {
        if *shutdown.borrow() {
            break;
        }
        let (enabled, bind, port) = manager.remote_config();
        let wanted = enabled && manager.remote_digest().is_some();
        if !wanted {
            if let Some(handle) = server.take() {
                handle.abort();
            }
            bound = None;
        } else if bound.as_ref() != Some(&(bind.clone(), port)) {
            if let Some(handle) = server.take() {
                handle.abort();
            }
            bound = None;
            let address = format!("{bind}:{port}");
            match tokio::net::TcpListener::bind(&address).await {
                Ok(listener) => {
                    tracing::info!(%address, "remote control listening");
                    bound = Some((bind, port));
                    server = Some(tokio::spawn(segmenta_daemon::remote::serve(
                        listener,
                        Arc::clone(&manager),
                        shutdown.clone(),
                    )));
                }
                Err(err) => tracing::warn!(
                    %address,
                    error = %err,
                    "remote control cannot bind; it stays off"
                ),
            }
        }
        tokio::select! {
            _ = shutdown.changed() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }
    }
    if let Some(handle) = server.take() {
        handle.abort();
    }
}

/// Publish the encrypted-DNS configuration the resolver reads.
fn publish_dns(store: &segmenta_daemon::store::Store) {
    let settings = store.settings().unwrap_or_default();
    let mode = segmenta_core::dns::DnsMode::parse(&settings.dns_mode).unwrap_or_default();
    let provider = segmenta_core::dns::DohProvider::parse(&settings.dns_provider)
        .unwrap_or_default()
        .url()
        .to_string();
    let endpoint = if settings.dns_custom_url.trim().is_empty() {
        provider
    } else {
        settings.dns_custom_url.trim().to_string()
    };
    segmenta_core::dns::set_config(Some(segmenta_core::dns::DnsConfig {
        mode,
        endpoint,
        interface: Some(settings.vpn_interface.clone()).filter(|name| !name.trim().is_empty()),
        allow_local: settings.allow_local_downloads,
    }));
}

async fn terminate_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    if let Ok(mut stream) = signal(SignalKind::terminate()) {
        stream.recv().await;
    } else {
        std::future::pending::<()>().await;
    }
}

fn set_mode(path: &std::path::Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|err| format!("cannot stat {}: {err}", path.display()))?
        .permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|err| format!("cannot chmod {}: {err}", path.display()))
}
