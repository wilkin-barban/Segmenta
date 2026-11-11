# Changelog

Todos los cambios notables de Segmenta se documentan en este archivo.

El formato sigue [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/) y el proyecto usa
[Versionado semántico](https://semver.org/lang/es/).

## [Unreleased]

## [0.0.2] - 2026-11-11

Release de correcciones del primer QA remoto sobre Ubuntu 26.04.1 LTS. El alcance no cambia:
sólo descargas directas HTTP/HTTPS.

### Added

- Auditoría y capturas: la preferencia `settings.appearance` (`system`|`light`|`dark`, por
  defecto `system`) sigue `AdwStyleManager`, y `segmenta-gui --screenshots <dir>` genera las
  capturas QA 1440×900 de ambos temas (`segmenta-light.png`, `segmenta-dark.png`).
- i18n: crate `segmenta-i18n` con catálogos completos en español, portugués de Brasil y inglés,
  detección por `LC_ALL`/`LC_MESSAGES`/`LANG` y fallback a inglés. Toda la GUI y los mensajes
  humanos de la CLI salen del catálogo; el protocolo JSON-RPC no cambia (los `code` siguen siendo
  la fuente y el texto se resuelve en el cliente).
- `segmenta-gui --version` y `segmenta-gui --help` (documenta `--smoke` y `--screenshots`).
- Logging estructurado en el daemon con `tracing`/`tracing-subscriber` a stderr (journald lo
  captura), filtro `RUST_LOG`/`SEGMENTA_LOG` con default `info`, campos de ciclo de vida
  (`task_id`, `attempt_id`, `state`, `status`, `error_code`, `retry`, `duration_ms`) y redacción
  estricta: URLs reducidas a `scheme://host/path`, sin cabeceras `Authorization`/`Cookie`, sin
  cuerpos, credenciales ni secretos.
- Dependencia `libsecret-tools` en el `.deb`: es la vía real al Secret Service del escritorio.
- `packaging/check-package.sh`, la puerta de empaquetado: valida el control, los diez archivos
  instalados, el `.desktop` y el metainfo extraídos del propio paquete, y ejecuta `lintian` si
  está disponible.

### Changed

- Backend de secretos: `KeyringSecretStore` se considera disponible por la existencia de
  `secret-tool` y una consulta de sondeo que devuelve 0 o 1 (nunca `--version`).
  `system.status.secret_backend` responde `keyring` o `unavailable` de verdad.
- El backend por defecto es el keyring o «no disponible»; ya no se elige `FileSecretStore` de
  forma automática. Un `download.add` con `Authorization`, `Cookie` o `cookies[]` sin backend
  falla con `1031` (`ERR_KEYRING_UNAVAILABLE`) sin crear la tarea, y una tarea existente cuyo
  secreto deja de poder leerse pasa a `paused` con `1031`.
- Migración del archivo legado: al arrancar con keyring disponible, `$XDG_DATA_HOME/segmenta/secrets.json`
  se importa al keyring (una entrada por alias) y el archivo se elimina; si el keyring no está
  disponible, el archivo se deja intacto, no se usa y se informa `unavailable`.
- Directorio de descargas XDG: `default_download_dir()` resuelve `XDG_DOWNLOAD_DIR`,
  `xdg-user-dir DOWNLOAD`, `~/.config/user-dirs.dirs` y recién entonces `$HOME/Downloads`. En
  Ubuntu español una descarga sin `-d` va a `~/Descargas`.
- `download_dir` histórico: si el valor persistido es exactamente el default antiguo
  (`$HOME/Downloads`), el usuario no lo eligió en `user-dirs.dirs` y XDG resuelve otra ruta, se
  migra una sola vez y queda registrado en el log. Un valor personalizado nunca se toca.
- `pause`/`resume` sólo se aceptan en estados válidos: `pause` en `downloading`, `pending`,
  `probing`, `retrying`, `verifying` y `finalizing`; `resume` en `paused` y `retrying`.
- Al pausar (o ante cualquier parada) el status de la tarea reporta `speed_bps = 0` y
  `eta_seconds = null`, igual que ya hacían los segmentos.
- AppStream: el metainfo usa sólo `<developer><name>` y `appstreamcli validate` no emite
  warnings.

### Fixed

- La GUI normal quedaba en «Conectando con el daemon…» con una tarea ya completada; ahora lista
  en menos de 2 s y, cuando el socket no está activo, muestra el motivo, el comando
  `systemctl --user enable --now segmenta.socket` y un botón **Reintentar** que reconecta de
  verdad y recarga la lista.
- `segmenta-gui --version` respondía «Opción desconocida».
- `pause`/`resume` sobre una tarea `failed` o terminal devolvían éxito mientras el estado seguía
  igual; ahora fallan con `1011` (`TASK_NOT_PAUSABLE`) y `1003` (`TASK_NOT_RESUMABLE`) sin
  cambiar el estado ni publicar eventos falsos.
- El status pausado conservaba `speed_bps` distinto de cero.
- `segmenta system status` informaba `secret_backend=file` aunque `secret-tool` funcionara.
- AppStream emitía `developer-name-tag-deprecated`.

## [0.0.1] - 2026-04-24

Primera release pública: gestor de descargas HTTP/HTTPS segmentado con partición en caliente y
reanudación tras caída, empaquetado para Ubuntu 26.04 LTS.

### Added

- Motor segmentado (`segmenta-core`): sondeo con `Range`/`Accept-Encoding: identity`,
  `Content-Range`, `ETag`/`Last-Modified` e `If-Range`; hasta 32 conexiones; fallback a un solo
  stream y a `chunked`; partición en caliente sin huecos ni solapamientos; `pwrite64` con
  `EINTR` y escrituras parciales; reintentos con backoff y `Retry-After`; límite de velocidad;
  checksum SHA-256 opcional.
- Daemon (`segmenta-daemon`): SQLite en WAL, migraciones idempotentes, secretos fuera de SQLite,
  socket `0600` con `SO_PEERCRED`, JSON-RPC 2.0 en NDJSON con eventos y unidades systemd de
  usuario con activación por socket.
- CLI (`segmenta`/`segmenta-cli`): `add`, `list`, `status`, `pause`, `resume`, `cancel`,
  `delete`, `settings`, `queues`, `hello`, `system`, `--json` y códigos de salida 0/1/2.
- GUI (`segmenta-gui`) GTK4 + Libadwaita con lista virtualizada, búsqueda, filtros, detalle con
  segmentos, alta con selector de carpeta, preferencias, notificaciones y atajos.
- Paquete `.deb` con los cuatro binarios, las unidades de usuario, `.desktop`, AppStream, icono
  y licencia.

[Unreleased]: https://github.com/wilkin-barban/Segmenta/compare/v0.0.2...HEAD
[0.0.2]: https://github.com/wilkin-barban/Segmenta/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1
