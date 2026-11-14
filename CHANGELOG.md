# Changelog

Todos los cambios notables de Segmenta se documentan en este archivo.

El formato sigue [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/) y el proyecto usa
[Versionado semántico](https://semver.org/lang/es/).

## [0.2.1] - 2026-11-13

Correcciones de interfaz, captura y motor, y gestión segura de descargas. Sin publicar: falta el
tag y el Release.

### Added

- **Limpiar completadas**: acción de menú que borra sólo filas `completed` y no toca ningún
  archivo; con cero completadas avisa y no hace nada.
- **Eliminar seleccionadas** (una o varias): selección múltiple con casilla por fila, diálogo que
  indica cuántas tareas y archivos se verán afectados y casilla «Eliminar también los archivos
  descargados» desmarcada por defecto. El diálogo no se cierra hasta conocer el resultado e informa
  filas borradas, archivos borrados y cada fallo por nombre.
- RPC `download.clean_completed` y `download.delete_many` con resultado por tarea; una tarea activa
  se detiene antes de tocar su archivo y un borrado imposible deja la fila `failed` con
  `1048 ERR_FILE_DELETE_FAILED`.

### Fixed

- **Motor**: un `200` con `Content-Range` válido que cubre el rango pedido ya no se interpreta
  como "el remoto cambió". iCloud responde exactamente así en un rango completo; antes la
  descarga se reiniciaba y terminaba en `1028 remote changed twice`.
- **Progreso de medios**: yt-dlp no siempre imprime el prefijo `download:` en la plantilla de
  progreso, y el parser lo exigía: no se publicaba ningún `task_progress` y la barra quedaba
  en 0 hasta completar. Ahora acepta ambas formas.
- **Captura**: un archivo pequeño que termina entre la pausa y la confirmación ya no queda en
  silencio; la ventana se abre con foco, se cierra si la descarga ya terminó y llega una
  notificación con el archivo, que al tocarla lo añade a Segmenta.
- **Gestión**: `delete_many` marca la tarea mientras la borra, de modo que ningún arranque de
  cola concurrente publica un `state_changed` después de la respuesta.
- **yt-dlp en el paquete**: el `.deb` embebe el zipapp oficial (versión fijada y SHA-256 en
  `packaging/ytdlp.lock`) y `locate_tool` lo prefiere sólo si es más nuevo que el del `PATH`.
  Ubuntu 26.04 trae `2026.03.17` y el mínimo es `2026.10.01`: sin esto YouTube no funcionaba
  out-of-the-box. `python3` y `ca-certificates` pasan a `Depends`.
- **CI**: workflow de GitHub Actions con fmt, clippy, ambas suites, build release, paquete y
  una instalación limpia en `ubuntu:26.04` que arranca la GUI bajo `xvfb`.

- **Preferencias** vuelve a mostrar sus siete páginas: la de Antivirus existía desde 0.2.0 pero
  nunca se añadía al diálogo. El diálogo abre a 960×760 y los títulos de fila ya no se convierten en
  una columna vertical de una letra por línea.
- El **spinner de carga** ya no aparece sobre la lista: la página de carga dejó de tener spinner y
  la ventana fija el cursor por defecto.
- **Diálogo de confirmación**: cerraba en falso al recibir la respuesta (`can_close(false)` también
  bloquea el `close()` programático) y dejaba la ventana sin responder; ahora se cierra y resetea su
  estado en éxito y en error.
- Preferencias: inset inferior para que el contenido no quede detrás de la barra de pestañas,
  buscador del diálogo retirado y descripciones de navegador con wrap.
- **Extensión / YouTube**: tras una navegación SPA el content script reporta `/` o el origen y
  `validateSender` lo rechazaba con `ERR_INVALID_SENDER`, mostrado como “La solicitud no es válida
  para la extensión”. Ahora acepta el origen exacto `https://www.youtube.com` y sigue validando el
  URL del payload por separado; `m.youtube.com`, HTTP, otros hosts y sufijos maliciosos siguen
  rechazados.
- **Extensión / allowlist de streams**: con `allowStreams` activo y allowlist vacía se aceptaba
  cualquier host; ahora es deny-by-default y sólo YouTube exacto o el host listado pasan.
- **Extensión / instalación real**: Chrome cargaba una copia 0.1.1 desde
  `~/.local/share/segmenta/extension/chromium`; el contrato de mensajes actual no coincide y el
  panel falla con el mismo texto. La copia se actualizó a 0.2.1 con backup de la anterior.
- **Lista de descargas**: las respuestas de `download.list` llegan en `created_at DESC`; el modelo
  las reinsertaba al principio y las invertía. Ahora conserva el orden del daemon, la tarea más
  nueva queda arriba y un `task_added` sigue entrando por delante.
- **Progreso real**: `yt-dlp` 2026.10.26 emite el template de progreso sin el prefijo `download:`;
  el parser lo exige y descartaba todas las líneas, dejando la barra en 0 hasta completar. Se
  aceptan ambas formas, con test de la línea real; para HLS/DASH o torrent sin tamaño conocido la
  barra usa modo indeterminado y muestra los bytes descargados.
- **Paquete Ubuntu 26.04+**: el `.deb` embebe el zipapp oficial de `yt-dlp` 2026.10.26 con SHA-256
  fijado, porque el paquete del archivo es anterior al mínimo de YouTube; declara `python3` y
  `ca-certificates`, y la instalación limpia en 26.04 se prueba en CI.

### Changed

- CI: workflow de GitHub Actions con fmt, clippy, Rust, Node, sync/pack, release build,
  `check-package`, instalación limpia en contenedor `ubuntu:26.04` y humo de GUI.

## [0.2.0] - 2026-11-12

0.2.0 publicada con QA real en Ubuntu y dos límites declarados (cold-start de captura y extensión
de Firefox no probados en navegador): scheduler de colas nativo con ventanas horarias y `max_active` real,
post-procesamiento argv seguro y ClamAV integrado con cuarentena, site logins Basic con GNOME
Keyring (sin Digest), sin categorías automáticas, extensión de instalación manual asistida por
la GUI, reconciliación de capturas al arrancar en frío, probe multiarchivo de torrents, ayuda CLI
con `extension` y documentación alineada. Ver
[`packaging/release-notes-0.2.0.md`](packaging/release-notes-0.2.0.md).

## [0.1.2] - 2026-11-12

Correcciones de estado real de la extensión, orden y foco de las tareas, panel de calidades
acotado y captura de archivos con confirmación. El alcance no cambia: descargas directas
HTTP/HTTPS, YouTube público y HLS/DASH públicos sin cifrar.

### Added

- `segmenta extension status|prepare|install-native|open`: prepara el ZIP en
  `$XDG_DATA_HOME/segmenta/extensions/<familia>-<versión>`, valida `manifest.json`, `_locales` e
  identificador, y abre `chrome://extensions`, `about:debugging` o la carpeta con argv fijo, sin
  shell. Nunca se escriben perfiles ni se usa `--load-extension` en Chrome estable.
- Ventana propia de confirmación para la captura: nombre, host y carpeta de destino, con
  `capture_confirm` (por defecto `true`) para volver al flujo sin ventana.
- Watchdog con `alarms` (permiso nuevo) que persiste cada captura pendiente antes de pausar y la
  rehidrata al despertar el service worker o al arrancar el navegador.
- Lista por defecto de extensiones capturadas ampliada con `deb`, `rpm`, `appimage`, `xz`, `whl`,
  `jar`, `epub`, `vsix`, `xpi` y `msix`.
- `download.add` y `torrent.add` devuelven `destination_dir` y `final_path`, y el evento
  `task_added` los incluye, para que la ventana muestre la carpeta efectiva.

### Changed

- La GUI ya no dice «Listo» sólo porque exista un manifiesto nativo: distingue manifiesto
  nativo listo, extensión no instalada, instalada y conectada, por navegador.
- `media.probe` deduplica los formatos por altura y expone 4K (2160), 1080p, 720p, 360p y solo
  audio, sólo los disponibles, con **720p** por defecto; el vídeo se selecciona por `height`,
  nunca por un `format_id` crudo, también en HLS/DASH.
- Los manifiestos por usuario respetan `XDG_CONFIG_HOME`, se escriben `0644`, validan el host y
  no sobrescriben un archivo existente en silencio.
- Una tarea nueva aparece al principio de la lista y recargar conserva el orden `created_at DESC`
  del daemon; `system.open` trae la GUI al frente y enfoca la tarea (`segmenta-gui --show-task`).
- Tras «Descargar», la tarjeta del panel pasa a «Enviado a Segmenta» con [Abrir Segmenta] y
  [Ver progreso].

### Fixed

- Una descarga `.zip`, `.rar`, `.deb` y compañía ya no se cancela antes de confirmar: se pausa,
  y aceptar crea la tarea en Segmenta antes de cancelar y borrar la copia del navegador.
- Rechazar, el timeout (~120 s), cerrar la ventana, un fallo del host o una descarga que no se
  puede pausar reanudan la descarga del navegador: no se pierde ningún archivo.
- El identificador del add-on de Firefox es el publicado, `segmenta@wilkin.github.io`, y un test
  lo ata al manifiesto real.

### Notes

- Una configuración escrita por 0.1.0 o 0.1.1 se carga igual: conserva los valores del usuario y
  gana los defaults nuevos. Sin cambios de esquema que rompan tareas existentes.

## [0.1.1] - 2026-11-11

Release de integración con navegadores, captura de descargas y torrents.

### Added

- Extensión de navegador con detección de Chrome, Chromium, Brave, Edge, Vivaldi, Opera y
  Firefox, y página **Navegadores** con estado del manifiesto nativo, instalación por usuario y
  fallback local. Los siete manifiestos nativos se instalan con el `.deb`.
- Captura de descargas con `downloads.onCreated` por extensión, denylist y cancelación de la
  descarga nativa; lista por defecto de 54 extensiones, editable en Preferencias, con `r0*`/`r1*`
  interpretados como prefijos.
- `.torrent` derivado a `torrent.add`; enlaces `magnet:` en el menú contextual y mediante un
  handler `.desktop`.
- Motor de torrents embebido `librqbit 9.1` con una sesión compartida por proceso, schema v3 con
  `torrents`/`torrent_files`, RPC y CLI `torrent probe|add|status`, progreso, cancelación y
  seeding desactivado por defecto. Saneamiento de rutas internas (`..`, absolutas, escapes).

### Changed

- El hover sobre una tarea ya no abre el detalle; sólo un clic real lo abre. «Atrás» limpia la
  selección y permite reabrir la tarea. Badge específico para torrents.

### Notes

- Limitaciones documentadas: cold-start del service worker, probe de `.torrent` multiarchivo y
  registro automático del handler del sistema según escritorio.
- Firefox snap y Flatpak no pueden ejecutar el host nativo.

## [0.1.0] - 2026-11-11

Primera release con descarga de medios y extensión de navegador. Mantiene el motor de descargas
directas de 0.0.2 y agrega YouTube, HLS/DASH públicos y un panel flotante para elegir calidad,
audio-only y playlists.

### Added

- Extensión de navegador: panel flotante en YouTube con título, miniatura, calidades de video,
  **Solo audio** y límite de playlist (1–500); targets Chromium MV3 y WebExtension MV3 para
  Firefox (`segmenta@wilkin.github.io`).
- Host nativo `segmenta-host` (Native Messaging, framing de 4 bytes LE, límite 1 MiB, sin shell)
  que reenvía `media.*` al daemon; el `.deb` instala los siete manifiestos nativos.
- `segmenta media probe|add` en la CLI y en el panel: YouTube video, audio-only y playlists con
  límite, y HLS/DASH públicos con `SegmentTemplate` descargados por segmentos y remuxeados con
  `ffmpeg -c copy`.
- Selector de video con fallback muxed/DASH
  `b[height<=H]/18/bv*[height<=H]+ba/b[height<=H]/best` y validación de la versión mínima de
  `yt-dlp` (`>= 2026.10.01`) antes de tocar la red.
- Nombres únicos por tarea (`… (1).m4a`) y `--no-continue` para no reanudar archivos ajenos;
  progreso persistido (bytes, total, velocidad, ETA) y tamaño final verificado.
- Paquetes reproducibles con `_locales` (es/en/pt-BR) validados por `extension/tools/pack.mjs`.

### Fixed

- Callback + Promise de `chrome.runtime.sendNativeMessage`, locales de la extensión, CLI/GUI de
  medios visible en `--help`, fallback de formatos de YouTube y errores 403/416, y probe de
  playlist acotado con `--playlist-end` para no fallar por entradas ocultas.
- Destino existente con `--continue`/`--no-part` que provocaba HTTP 416.

### Notes

- DASH con `SegmentList`/`SegmentBase` queda fuera de alcance (`1043`); DRM, AES-128 y streams
  en vivo se rechazan explícitamente. Sin subtítulos ni multi-audio.
- Requisitos: Ubuntu 26.04, `ffmpeg` y `yt-dlp >= 2026.10.01`. El `.deb` es el único formato de
  instalación; el snap de Firefox no permite la conexión remota de `web-ext`.

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

[Unreleased]: https://github.com/wilkin-barban/Segmenta/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/wilkin-barban/Segmenta/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/wilkin-barban/Segmenta/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/wilkin-barban/Segmenta/compare/v0.0.2...v0.1.0
[0.0.2]: https://github.com/wilkin-barban/Segmenta/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1
