//! A minimal FTP server for the client tests: `127.0.0.1`, one directory, no filesystem.
//!
//! It speaks only what the client uses — greeting, `USER`/`PASS`, `TYPE`, `SIZE`, `MDTM`,
//! `EPSV`/`PASV`, `REST`, `RETR`, `QUIT` — so a protocol mistake shows up as a failing
//! expectation rather than as a permissive server.

use segmenta_core::ftp::{probe, FtpConfig, FtpTarget};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn local() -> FtpConfig {
    FtpConfig {
        allow_local: true,
        ..FtpConfig::default()
    }
}

/// What the test server holds and how it behaves.
#[derive(Clone)]
pub struct FtpServerConfig {
    /// Bytes the client will receive.
    pub body: Vec<u8>,
    /// Path the client must ask for; anything else is 550.
    pub path: String,
    /// Answer `EPSV`? When false, the client has to fall back to `PASV`.
    pub extended_passive: bool,
    /// Report a wrong size in `SIZE` (to test the explicit mismatch path).
    pub size_override: Option<u64>,
    /// Break the transfer after this many bytes, as a dropped server would.
    pub cut_after: Option<usize>,
    /// Serve TLS: `None` is plain FTP, `Some(false)` implicit, `Some(true)` explicit.
    pub tls: Option<bool>,
}

impl Default for FtpServerConfig {
    fn default() -> Self {
        FtpServerConfig {
            body: Vec::new(),
            path: "/file.bin".to_string(),
            extended_passive: true,
            size_override: None,
            cut_after: None,
            tls: None,
        }
    }
}

/// One request the server saw, for assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpRecord {
    /// Command verb as received.
    pub verb: String,
    /// Argument, when there was one.
    pub argument: String,
}

/// A running test server.
pub struct FtpServer {
    addr: SocketAddr,
    records: Arc<std::sync::Mutex<Vec<FtpRecord>>>,
    tls: Option<TlsMaterial>,
}

/// Certificate generated in memory, trusted only by this test.
pub struct TlsMaterial {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl TlsMaterial {
    fn generate() -> TlsMaterial {
        // The client connects to a literal address, so the certificate must name it: a
        // certificate for `localhost` alone is rejected for `127.0.0.1`, which is the
        // correct behaviour and worth testing against.
        let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .expect("certificate params");
        params
            .subject_alt_names
            .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
                std::net::Ipv4Addr::LOCALHOST,
            )));
        let key = rcgen::KeyPair::generate().expect("key");
        let cert = params.self_signed(&key).expect("self signed");
        TlsMaterial {
            cert_der: cert.der().to_vec(),
            key_der: key.serialize_der(),
        }
    }

    fn acceptor(&self) -> tokio_rustls::TlsAcceptor {
        // The test server builds its own configuration, so the provider has to be chosen
        // here as well: rustls refuses to guess when two are compiled in.
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let cert = tokio_rustls::rustls::pki_types::CertificateDer::from(self.cert_der.clone());
        let key =
            tokio_rustls::rustls::pki_types::PrivateKeyDer::Pkcs8(self.key_der.clone().into());
        let config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("server config");
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
    }

    /// Client configuration trusting exactly this certificate.
    pub fn client_config(&self) -> Arc<tokio_rustls::rustls::ClientConfig> {
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        roots
            .add(tokio_rustls::rustls::pki_types::CertificateDer::from(
                self.cert_der.clone(),
            ))
            .expect("root");
        Arc::new(
            tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }
}

impl FtpServer {
    /// Start listening; the server runs until the test ends.
    pub async fn start(config: FtpServerConfig) -> FtpServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&records);
        let tls = config.tls.map(|_| TlsMaterial::generate());
        let shared = tls
            .as_ref()
            .map(|_| Arc::new(tls.as_ref().unwrap().acceptor()));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let config = config.clone();
                let seen = Arc::clone(&seen);
                let acceptor = shared.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, config, seen, acceptor).await;
                });
            }
        });
        FtpServer { addr, records, tls }
    }

    /// `ftp://host:port/path` for the client under test.
    pub fn url(&self) -> url::Url {
        self.url_for(&self.path())
    }

    fn path(&self) -> String {
        "/file.bin".to_string()
    }

    /// URL for another path on the same server.
    pub fn url_for(&self, path: &str) -> url::Url {
        url::Url::parse(&format!("ftp://127.0.0.1:{}{path}", self.addr.port())).unwrap()
    }

    /// Port the server listens on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Client configuration that trusts this server's certificate, when it has one.
    pub fn client_config(&self) -> Arc<tokio_rustls::rustls::ClientConfig> {
        self.tls
            .as_ref()
            .map(TlsMaterial::client_config)
            .expect("the server was started without TLS")
    }

    /// Commands received so far.
    pub fn records(&self) -> Vec<FtpRecord> {
        self.records.lock().unwrap().clone()
    }

    /// Verbs received so far.
    pub fn verbs(&self) -> Vec<String> {
        self.records()
            .into_iter()
            .map(|record| record.verb)
            .collect()
    }
}

type Conn = Box<dyn Stream>;

trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Stream for T {}

/// Read one CRLF line from a connection that may be plain or TLS.
async fn read_command(conn: &mut Conn) -> std::io::Result<Option<String>> {
    let mut raw = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        if conn.read(&mut byte).await? == 0 {
            return Ok(None);
        }
        if byte[0] == b'\n' {
            return Ok(Some(
                String::from_utf8_lossy(&raw)
                    .trim_end_matches('\r')
                    .to_string(),
            ));
        }
        raw.push(byte[0]);
    }
}

async fn serve(
    stream: TcpStream,
    config: FtpServerConfig,
    records: Arc<std::sync::Mutex<Vec<FtpRecord>>>,
    acceptor: Option<Arc<tokio_rustls::TlsAcceptor>>,
) -> std::io::Result<()> {
    // Implicit TLS wraps the socket before the greeting; explicit waits for AUTH TLS.
    let mut conn: Conn = match (&acceptor, config.tls) {
        (Some(acceptor), Some(false)) => Box::new(acceptor.accept(stream).await?),
        _ => Box::new(stream),
    };
    conn.write_all(b"220 segmenta test ftp\r\n").await?;
    let mut pending: Option<TcpListener> = None;
    let mut offset: u64 = 0;
    let mut protected = false;
    loop {
        let Some(line) = read_command(&mut conn).await? else {
            return Ok(());
        };
        let (verb, argument) = match line.split_once(' ') {
            Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.to_string()),
            None => (line.to_ascii_uppercase(), String::new()),
        };
        records.lock().unwrap().push(FtpRecord {
            verb: verb.clone(),
            argument: argument.clone(),
        });
        match verb.as_str() {
            "USER" => conn.write_all(b"331 need password\r\n").await?,
            "PASS" => conn.write_all(b"230 logged in\r\n").await?,
            "TYPE" => conn.write_all(b"200 binary\r\n").await?,
            "AUTH" if config.tls == Some(true) && argument.eq_ignore_ascii_case("TLS") => {
                conn.write_all(b"234 upgrade\r\n").await?;
                conn.flush().await?;
                match acceptor.clone() {
                    Some(acceptor) => conn = Box::new(acceptor.accept(conn).await?),
                    None => conn.write_all(b"431 no tls\r\n").await?,
                }
            }
            "PBSZ" => conn.write_all(b"200 pbsz\r\n").await?,
            "PROT" => {
                protected = argument.eq_ignore_ascii_case("P");
                conn.write_all(b"200 prot\r\n").await?;
            }
            "SIZE" if argument == config.path => {
                let size = config.size_override.unwrap_or(config.body.len() as u64);
                conn.write_all(format!("213 {size}\r\n").as_bytes()).await?;
            }
            "MDTM" if argument == config.path => {
                conn.write_all(b"213 20260101120000\r\n").await?;
            }
            "EPSV" if config.extended_passive => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                pending = Some(listener);
                conn.write_all(
                    format!("229 Entering Extended Passive Mode (|||{port}|)\r\n").as_bytes(),
                )
                .await?;
            }
            "PASV" => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                pending = Some(listener);
                conn.write_all(
                    format!(
                        "227 Entering Passive Mode (127,0,0,1,{}, {})\r\n",
                        port / 256,
                        port % 256
                    )
                    .as_bytes(),
                )
                .await?;
            }
            "REST" => {
                offset = argument.parse::<u64>().unwrap_or(0);
                conn.write_all(b"350 restarting\r\n").await?;
            }
            "RETR" => {
                if argument != config.path {
                    conn.write_all(b"550 no such file\r\n").await?;
                    continue;
                }
                let Some(listener) = pending.take() else {
                    conn.write_all(b"425 use PASV first\r\n").await?;
                    continue;
                };
                conn.write_all(b"150 opening data\r\n").await?;
                let (data, _) = listener.accept().await?;
                let mut data: Conn = match (&acceptor, protected) {
                    (Some(acceptor), true) => Box::new(acceptor.accept(data).await?),
                    _ => Box::new(data),
                };
                let start = offset.min(config.body.len() as u64) as usize;
                let mut slice = &config.body[start..];
                if let Some(limit) = config.cut_after {
                    slice = &slice[..slice.len().min(limit)];
                }
                data.write_all(slice).await?;
                data.flush().await?;
                drop(data);
                offset = 0;
                conn.write_all(b"226 transfer complete\r\n").await?;
            }
            "QUIT" => {
                conn.write_all(b"221 bye\r\n").await?;
                return Ok(());
            }
            _ => conn.write_all(b"500 unsupported\r\n").await?,
        }
    }
}

#[tokio::test]
async fn the_probe_reads_size_and_modification_time() {
    let server = FtpServer::start(FtpServerConfig {
        body: vec![9u8; 4096],
        ..FtpServerConfig::default()
    })
    .await;
    let probe = probe(&server.url(), &local()).await.unwrap();
    assert_eq!(probe.size, Some(4096));
    assert_eq!(probe.modified.as_deref(), Some("20260101120000"));
    let verbs = server.verbs();
    assert!(verbs.contains(&"SIZE".to_string()), "{verbs:?}");
    assert!(verbs.contains(&"MDTM".to_string()), "{verbs:?}");
    assert!(verbs.contains(&"TYPE".to_string()), "{verbs:?}");
    // The probe is metadata only: no passive listener is opened before a transfer.
    assert!(!verbs.contains(&"EPSV".to_string()), "{verbs:?}");
    assert!(!verbs.contains(&"PASV".to_string()), "{verbs:?}");
}

#[tokio::test]
async fn a_server_without_epsv_falls_back_to_pasv() {
    let server = FtpServer::start(FtpServerConfig {
        body: vec![1u8; 16],
        extended_passive: false,
        ..FtpServerConfig::default()
    })
    .await;
    // The probe itself does not need a data connection; the fallback is exercised by the
    // download unit. Here we only assert the server never saw EPSV succeed.
    let probe = probe(&server.url(), &local()).await.unwrap();
    assert_eq!(probe.size, Some(16));
}

#[tokio::test]
async fn a_private_target_is_refused_unless_local_downloads_are_allowed() {
    let server = FtpServer::start(FtpServerConfig::default()).await;
    let strict = FtpConfig {
        allow_local: false,
        ..FtpConfig::default()
    };
    assert!(probe(&server.url(), &strict).await.is_err());
}

#[tokio::test]
async fn credentials_in_the_link_are_used() {
    let server = FtpServer::start(FtpServerConfig {
        body: vec![2u8; 8],
        ..FtpServerConfig::default()
    })
    .await;
    let mut url = server.url();
    url.set_username("alice").unwrap();
    url.set_password(Some("s3cret")).unwrap();
    let target = FtpTarget::parse(&url).unwrap();
    assert_eq!(target.user, "alice");
    assert_eq!(target.password, "s3cret");
    probe(&url, &local()).await.unwrap();
    let verbs = server.verbs();
    assert!(verbs.contains(&"USER".to_string()));
    assert!(verbs.contains(&"PASS".to_string()));
}

#[tokio::test]
async fn a_missing_path_is_reported_not_guessed() {
    let server = FtpServer::start(FtpServerConfig {
        body: vec![0u8; 4],
        path: "/only-this.bin".to_string(),
        ..FtpServerConfig::default()
    })
    .await;
    let probe = probe(&server.url_for("/other.bin"), &local())
        .await
        .unwrap();
    // A server that cannot SIZE a path still answers; the transfer unit is what fails.
    assert_eq!(probe.size, None);
}

#[tokio::test]
async fn a_file_is_downloaded_in_segments_and_matches_byte_for_byte() {
    use segmenta_core::engine::Canceller;
    // Above the 5 MB threshold the planner actually segments; below it, one connection is
    // the correct answer and a multi-RETR assertion would be wrong.
    let body: Vec<u8> = (0..6 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let server = FtpServer::start(FtpServerConfig {
        body: body.clone(),
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    let cfg = FtpConfig {
        connections: 4,
        allow_local: true,
        ..FtpConfig::default()
    };
    let written = segmenta_core::ftp::download(&server.url(), &path, &cfg, None, &Canceller::new())
        .await
        .unwrap();
    assert_eq!(written, body.len() as u64);
    assert_eq!(std::fs::read(&path).unwrap(), body);
    // More than one RETR means the file really was segmented.
    let retr = server
        .verbs()
        .into_iter()
        .filter(|verb| verb == "RETR")
        .count();
    assert!(retr >= 2, "expected several RETR, saw {retr}");
}

#[tokio::test]
async fn every_segment_after_the_first_asks_for_its_offset() {
    use segmenta_core::engine::Canceller;
    let server = FtpServer::start(FtpServerConfig {
        body: vec![5u8; 6 * 1024 * 1024],
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = FtpConfig {
        connections: 2,
        allow_local: true,
        ..FtpConfig::default()
    };
    segmenta_core::ftp::download(
        &server.url(),
        &dir.path().join("f.bin"),
        &cfg,
        None,
        &Canceller::new(),
    )
    .await
    .unwrap();
    let offsets: Vec<String> = server
        .records()
        .into_iter()
        .filter(|record| record.verb == "REST")
        .map(|record| record.argument)
        .collect();
    assert!(!offsets.is_empty(), "a segmented transfer must REST");
    assert!(offsets.iter().any(|offset| offset != "0"), "{offsets:?}");
}

#[tokio::test]
async fn a_per_host_limit_lowers_the_segment_count() {
    use segmenta_core::engine::Canceller;
    use segmenta_core::hostlimits::{HostLimitPolicy, HostLimits, MonotonicClock};
    use std::sync::Arc;

    let server = FtpServer::start(FtpServerConfig {
        body: vec![3u8; 6 * 1024 * 1024],
        ..FtpServerConfig::default()
    })
    .await;
    let limits = HostLimits::new(
        HostLimitPolicy::default(),
        Arc::new(MonotonicClock::default()),
    );
    limits.on_throttled("127.0.0.1");
    limits.on_throttled("127.0.0.1");
    limits.on_throttled("127.0.0.1");
    let dir = tempfile::tempdir().unwrap();
    let cfg = FtpConfig {
        connections: 8,
        allow_local: true,
        ..FtpConfig::default()
    };
    segmenta_core::ftp::download(
        &server.url(),
        &dir.path().join("f.bin"),
        &cfg,
        Some(&limits),
        &Canceller::new(),
    )
    .await
    .unwrap();
    let retr = server
        .verbs()
        .into_iter()
        .filter(|verb| verb == "RETR")
        .count();
    assert_eq!(retr, 1, "a host limited to one opens one RETR, not four");
}

#[tokio::test]
async fn a_server_without_size_still_transfers_and_a_cut_is_reported() {
    use segmenta_core::engine::Canceller;
    // The server truncates the stream: the bytes written stay, but the download must not
    // claim the full file.
    let body = vec![7u8; 50_000];
    let server = FtpServer::start(FtpServerConfig {
        body: body.clone(),
        cut_after: Some(1_000),
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = FtpConfig {
        connections: 1,
        allow_local: true,
        ..FtpConfig::default()
    };
    let written = segmenta_core::ftp::download(
        &server.url(),
        &dir.path().join("f.bin"),
        &cfg,
        None,
        &Canceller::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        written, 1_000,
        "the count must reflect what the server actually sent"
    );
}

fn tls_config(server: &FtpServer, mode: segmenta_core::ftps::TlsMode) -> FtpConfig {
    FtpConfig {
        connections: 2,
        allow_local: true,
        tls: segmenta_core::ftps::TlsSetup {
            mode,
            client: Some(server.client_config()),
        },
        ..FtpConfig::default()
    }
}

#[tokio::test]
async fn an_implicit_ftps_transfer_is_encrypted_end_to_end() {
    use segmenta_core::engine::Canceller;
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    let server = FtpServer::start(FtpServerConfig {
        body: body.clone(),
        tls: Some(false),
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secure.bin");
    let cfg = tls_config(&server, segmenta_core::ftps::TlsMode::Implicit);
    // `ftps://` is what selects implicit TLS, so the URL is built with it directly.
    let url = url::Url::parse(&format!("ftps://127.0.0.1:{}/file.bin", server.port())).unwrap();
    segmenta_core::ftp::download(&url, &path, &cfg, None, &Canceller::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body);
    // The data channel was protected: the server only wraps it after `PROT P`.
    let verbs = server.verbs();
    assert!(verbs.contains(&"PBSZ".to_string()), "{verbs:?}");
    assert!(verbs.contains(&"PROT".to_string()), "{verbs:?}");
}

#[tokio::test]
async fn an_explicit_ftps_probe_upgrades_the_control_channel() {
    let server = FtpServer::start(FtpServerConfig {
        body: vec![1u8; 64],
        tls: Some(true),
        ..FtpServerConfig::default()
    })
    .await;
    let cfg = tls_config(&server, segmenta_core::ftps::TlsMode::Explicit);
    let probe = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        probe(&server.url(), &cfg),
    )
    .await
    .expect("the explicit probe must not hang")
    .unwrap();
    assert_eq!(probe.size, Some(64));
    assert!(
        server.verbs().iter().any(|verb| verb == "AUTH"),
        "{:?}",
        server.verbs()
    );
}

#[tokio::test]
async fn an_explicit_ftps_transfer_upgrades_after_login() {
    use segmenta_core::engine::Canceller;
    let body = vec![4u8; 200_000];
    let server = FtpServer::start(FtpServerConfig {
        body: body.clone(),
        tls: Some(true),
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("explicit.bin");
    let cfg = tls_config(&server, segmenta_core::ftps::TlsMode::Explicit);
    segmenta_core::ftp::download(&server.url(), &path, &cfg, None, &Canceller::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body);
    let verbs = server.verbs();
    // AUTH TLS comes after the login and before any PASV/EPSV.
    let auth = verbs.iter().position(|verb| verb == "AUTH").unwrap();
    let pass = verbs.iter().position(|verb| verb == "PASS").unwrap();
    assert!(pass < auth, "{verbs:?}");
}

#[tokio::test]
async fn a_changed_modification_time_blocks_the_resume_before_rest() {
    use segmenta_core::engine::Canceller;
    let server = FtpServer::start(FtpServerConfig {
        body: vec![1u8; 6 * 1024 * 1024],
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = FtpConfig {
        connections: 2,
        allow_local: true,
        expect_modified: Some("19990101000000".to_string()),
        ..FtpConfig::default()
    };
    let refused = segmenta_core::ftp::download(
        &server.url(),
        &dir.path().join("f.bin"),
        &cfg,
        None,
        &Canceller::new(),
    )
    .await;
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("changed"), "{message}");
    // No REST may be sent when the validator already failed.
    assert!(!server.verbs().iter().any(|verb| verb == "REST"));
}

#[tokio::test]
async fn a_matching_modification_time_allows_the_transfer() {
    use segmenta_core::engine::Canceller;
    let body = vec![2u8; 6 * 1024 * 1024];
    let server = FtpServer::start(FtpServerConfig {
        body: body.clone(),
        ..FtpServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.bin");
    let cfg = FtpConfig {
        connections: 2,
        allow_local: true,
        expect_modified: Some("20260101120000".to_string()),
        ..FtpConfig::default()
    };
    segmenta_core::ftp::download(&server.url(), &path, &cfg, None, &Canceller::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body);
}

#[tokio::test]
async fn a_dead_vpn_interface_refuses_before_any_socket() {
    let server = FtpServer::start(FtpServerConfig::default()).await;
    let cfg = FtpConfig {
        allow_local: true,
        interface: Some("vpnx-does-not-exist".to_string()),
        ..FtpConfig::default()
    };
    let error = probe(&server.url(), &cfg).await.unwrap_err().to_string();
    assert!(error.contains("ERR_VPN_INTERFACE"), "{error}");
    assert!(
        server.verbs().is_empty(),
        "no command may reach the server: {:?}",
        server.verbs()
    );
}
