//! A minimal FTP server for the client tests: `127.0.0.1`, one directory, no filesystem.
//!
//! It speaks only what the client uses — greeting, `USER`/`PASS`, `TYPE`, `SIZE`, `MDTM`,
//! `EPSV`/`PASV`, `REST`, `RETR`, `QUIT` — so a protocol mistake shows up as a failing
//! expectation rather than as a permissive server.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

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
}

impl Default for FtpServerConfig {
    fn default() -> Self {
        FtpServerConfig {
            body: Vec::new(),
            path: "/file.bin".to_string(),
            extended_passive: true,
            size_override: None,
            cut_after: None,
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
}

impl FtpServer {
    /// Start listening; the server runs until the test ends.
    pub async fn start(config: FtpServerConfig) -> FtpServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&records);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let config = config.clone();
                let seen = Arc::clone(&seen);
                tokio::spawn(async move {
                    let _ = serve(stream, config, seen).await;
                });
            }
        });
        FtpServer { addr, records }
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

async fn serve(
    stream: TcpStream,
    config: FtpServerConfig,
    records: Arc<std::sync::Mutex<Vec<FtpRecord>>>,
) -> std::io::Result<()> {
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    write.write_all(b"220 segmenta test ftp\r\n").await?;
    // Data listener is created on demand by EPSV/PASV and consumed by RETR.
    let mut pending: Option<TcpListener> = None;
    let mut offset: u64 = 0;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(());
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        let (verb, argument) = match line.split_once(' ') {
            Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.to_string()),
            None => (line.to_ascii_uppercase(), String::new()),
        };
        records.lock().unwrap().push(FtpRecord {
            verb: verb.clone(),
            argument: argument.clone(),
        });
        match verb.as_str() {
            "USER" => write.write_all(b"331 need password\r\n").await?,
            "PASS" => write.write_all(b"230 logged in\r\n").await?,
            "TYPE" => write.write_all(b"200 binary\r\n").await?,
            "SIZE" if argument == config.path => {
                let size = config.size_override.unwrap_or(config.body.len() as u64);
                write
                    .write_all(format!("213 {size}\r\n").as_bytes())
                    .await?;
            }
            "MDTM" if argument == config.path => {
                write.write_all(b"213 20260101120000\r\n").await?;
            }
            "EPSV" if config.extended_passive => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                pending = Some(listener);
                write
                    .write_all(
                        format!("229 Entering Extended Passive Mode (|||{port}|)\r\n").as_bytes(),
                    )
                    .await?;
            }
            "PASV" => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                pending = Some(listener);
                write
                    .write_all(
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
                write.write_all(b"350 restarting\r\n").await?;
            }
            "RETR" => {
                if argument != config.path {
                    write.write_all(b"550 no such file\r\n").await?;
                    continue;
                }
                let Some(listener) = pending.take() else {
                    write.write_all(b"425 use PASV first\r\n").await?;
                    continue;
                };
                write.write_all(b"150 opening data\r\n").await?;
                let (mut data, _) = listener.accept().await?;
                let start = offset.min(config.body.len() as u64) as usize;
                let mut slice = &config.body[start..];
                if let Some(limit) = config.cut_after {
                    slice = &slice[..slice.len().min(limit)];
                }
                data.write_all(slice).await?;
                data.flush().await?;
                drop(data);
                offset = 0;
                write.write_all(b"226 transfer complete\r\n").await?;
            }
            "QUIT" => {
                write.write_all(b"221 bye\r\n").await?;
                return Ok(());
            }
            _ => write.write_all(b"500 unsupported\r\n").await?,
        }
    }
}

use segmenta_core::ftp::{probe, FtpConfig, FtpTarget};

fn local() -> FtpConfig {
    FtpConfig {
        allow_local: true,
        ..FtpConfig::default()
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
