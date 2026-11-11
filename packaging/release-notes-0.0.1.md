# Segmenta 0.0.1

Primera release pública de Segmenta: gestor de descargas HTTP/HTTPS segmentado, con partición
en caliente y reanudación tras caída, para **Ubuntu 26.04 LTS (GNOME 50 / Wayland)**.

## Qué incluye

- **Motor segmentado** (`segmenta-core`): sondeo con `Range`/`Accept-Encoding: identity`,
  `Content-Range`, `ETag`/`Last-Modified` e `If-Range`; hasta 32 conexiones; fallback a un solo
  stream y a `chunked`; partición en caliente con `SplitRequest`/`SplitAck` sin huecos ni
  solapamientos; `pwrite64` con `EINTR`/escrituras parciales y `posix_fallocate` con fallback;
  reintentos con backoff y `Retry-After`; límite de velocidad global; checksum SHA-256 opcional.
- **Daemon** (`segmenta-daemon`): SQLite WAL con el DDL de OpenSpec 08, migraciones idempotentes,
  `next_offset` por segmento, secretos fuera de SQLite, socket `0600` con `SO_PEERCRED`, JSON-RPC
  2.0 en NDJSON con límite de 1 MiB y eventos con `seq_id`. Unidades systemd de usuario con
  socket activation.
- **CLI** (`segmenta` y `segmenta-cli`): `add`, `list`, `status`, `pause`, `resume`, `cancel`,
  `delete`, `settings`, `queues`, `hello`, `system`, `--json`, prefijos de id y códigos de salida
  0/1/2.
- **GUI** (`segmenta-gui`): cliente GTK4 + Libadwaita con `AdwNavigationSplitView`, lista
  virtualizada con búsqueda y filtros, detalle con vista de segmentos, alta con selector de carpeta
  del portal, preferencias (carpeta, conexiones, límite de velocidad, reintentos, proxy, autostart),
  notificaciones, tema claro/oscuro y atajos `Ctrl+N/F/,/Q`.
- **Paquete `.deb`** con los cuatro binarios, las unidades de usuario, `.desktop`, AppStream, icono
  y licencia.

## Instalación

```bash
sha256sum -c segmenta_0.0.1-1_amd64.deb.sha256
sudo apt install ./segmenta_0.0.1-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
```

Requiere `libgtk-4-1` y `libadwaita-1-0` (declaradas como dependencias). `ffmpeg` es sólo una
recomendación: lo usará el muxer HLS/DASH de 0.1.0.

Primer uso:

```bash
segmenta add https://example.com/ubuntu.iso
segmenta list
segmenta status <id>
segmenta pause <id> && segmenta resume <id>
segmenta settings set max_connections 8 global_speed_limit_bps 2000000
```

La GUI abre con `segmenta-gui` o la entrada **Segmenta** del menú.

## Verificación de esta release

Ejecutado en Ubuntu 26.04.1 LTS (GTK 4.22.4, libadwaita 1.9.1, Rust 1.99.1):

- `cargo test --workspace` — 151 tests, 0 fallos, incluido el e2e de la CLI contra un daemon real.
- `cargo fmt --all -- --check` y `cargo clippy --workspace --all-targets -- -D warnings` — limpios.
- `xvfb-run -a cargo test -p segmenta-gui` — las 5 pruebas de ventana construyen widgets reales.
- `xvfb-run -a segmenta-gui --smoke` — `SMOKE OK rows=1 segments=4`, 5 de 5 corridas.
- Descarga de 10 MiB con 8 conexiones ⇒ SHA-256 idéntico al origen.
- 32 MiB: pausa a ~18 MiB, `SIGKILL` al daemon, reconexión (socket activation), reanudación y
  SHA-256 idéntico al origen.
- Dos descargas simultáneas ⇒ ambas completas, checksums correctos, sin errores de checkpoint.
- `.deb` instalado con `dpkg -i`; `segmenta.socket` activo; `segmenta hello` responde;
  `segmenta-daemon` arranca por systemd de usuario y sobrevive a un reinicio del servicio.

## Limitaciones conocidas

- Sólo descargas directas: **sin HLS/DASH, sin DRM, sin subtítulos ni multi-audio** (0.1.0).
- **Sin extensión de navegador** ni Native Messaging; el esqueleto de tablas queda listo.
- `queue.schedule` persiste la configuración pero **no arranca ni detiene tareas por horario**, y
  `max_active` no limita la concurrencia todavía.
- Si un servidor ignora `Accept-Encoding: identity` y comprime, la descarga falla con
  `ERR_UNSUPPORTED` en lugar de escribir bytes comprimidos.
- Secretos en el Secret Service cuando `secret-tool` está disponible; si no, archivo `0600` aparte
  (nunca en SQLite).
- Sin catálogos gettext: la UI está en español.
- `.deb` es el único formato de esta release (sin Flatpak/Snap/AppImage).

## Checksum

Ver `segmenta_0.0.1-1_amd64.deb.sha256` junto al paquete.
