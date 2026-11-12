-- Normative schema of OpenSpec 08 (DDL verbatim). Applied once inside a transaction
-- by Store::migrate; the schema version lives in PRAGMA user_version.

CREATE TABLE IF NOT EXISTS queues (
    name TEXT PRIMARY KEY,
    max_active INTEGER NOT NULL DEFAULT 1 CHECK(max_active BETWEEN 1 AND 8),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT OR IGNORE INTO queues(name, max_active) VALUES ('main', 3);

CREATE TABLE IF NOT EXISTS downloads (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    final_url TEXT,
    final_path TEXT NOT NULL,
    temp_path TEXT,
    total_bytes INTEGER, -- NULL para transfer-encoding: chunked
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'probing', 'downloading', 'paused',
        'retrying', 'verifying', 'finalizing',
        'completed', 'failed', 'cancelled'
    )),
    supports_ranges BOOLEAN NOT NULL DEFAULT 0,
    etag TEXT,
    last_modified TEXT,
    content_type TEXT,
    expected_checksum TEXT,
    error_code INTEGER,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    priority INTEGER NOT NULL DEFAULT 0,
    queue_name TEXT NOT NULL DEFAULT 'main' REFERENCES queues(name),
    download_type TEXT NOT NULL DEFAULT 'direct' CHECK(download_type IN ('direct', 'hls', 'dash', 'youtube_video', 'youtube_playlist', 'youtube_audio', 'torrent', 'magnet')),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    -- Resultado de la fase de cierre (hook y antivirus) y ruta de cuarentena (v4).
    postprocess_exit INTEGER,
    postprocess_message TEXT,
    quarantine_path TEXT
);

CREATE TABLE IF NOT EXISTS download_headers (
    download_id TEXT PRIMARY KEY,
    -- Sólo cabeceras no sensibles (User-Agent, Referer, Accept, etc.).
    headers_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS download_secrets (
    download_id TEXT PRIMARY KEY,
    -- Alias/ID del secreto en GNOME Keyring; contiene Authorization y cookies.
    keyring_secret_id TEXT NOT NULL,
    FOREIGN KEY(download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    download_id TEXT NOT NULL,
    start_offset INTEGER NOT NULL,
    end_offset INTEGER NOT NULL,
    next_offset INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('idle', 'active', 'completed', 'failed')),
    CHECK(start_offset >= 0),
    CHECK(end_offset >= start_offset),
    CHECK(next_offset >= start_offset AND next_offset <= end_offset + 1),
    UNIQUE(download_id, start_offset),
    FOREIGN KEY(download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS download_stream_plan (
    download_id TEXT PRIMARY KEY,
    manifest_url TEXT NOT NULL,
    stream_kind TEXT NOT NULL CHECK(stream_kind IN ('hls', 'dash')),
    selected_variant TEXT NOT NULL,
    init_video_url TEXT,
    init_audio_url TEXT,
    media_sequence INTEGER NOT NULL DEFAULT 0,
    fragments_json TEXT NOT NULL,
    FOREIGN KEY(download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS site_logins (
    domain TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    keyring_secret_id TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS torrents (
    download_id TEXT PRIMARY KEY,
    info_hash TEXT NOT NULL UNIQUE,
    source TEXT NOT NULL,
    seed_after_complete BOOLEAN NOT NULL DEFAULT 0,
    uploaded_bytes INTEGER NOT NULL DEFAULT 0,
    peers INTEGER NOT NULL DEFAULT 0,
    files_count INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS torrent_files (
    download_id TEXT NOT NULL,
    file_index INTEGER NOT NULL,
    path TEXT NOT NULL,
    length INTEGER NOT NULL,
    selected BOOLEAN NOT NULL DEFAULT 1,
    PRIMARY KEY(download_id, file_index),
    FOREIGN KEY(download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

-- Índices para consultas de alta velocidad en la GUI
CREATE INDEX IF NOT EXISTS idx_downloads_status ON downloads(status);
CREATE INDEX IF NOT EXISTS idx_downloads_queue ON downloads(queue_name, priority DESC);
CREATE INDEX IF NOT EXISTS idx_segments_download_id ON segments(download_id);
CREATE INDEX IF NOT EXISTS idx_downloads_created_at ON downloads(created_at DESC);
