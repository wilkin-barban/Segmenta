# Changelog

Todos los cambios notables de Segmenta se documentan en este archivo.

El formato sigue [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/) y el proyecto usa
[Versionado semántico](https://semver.org/lang/es/).

## [0.3.0] - 2026-11-15

Velocidad real visible en la fila de descargas y en torrents/streams, descargas programadas para una
fecha y hora exactas, icono en la bandeja del sistema con menú adaptado a Ubuntu y ciclo de vida de
cierre a bandeja. Publicada como `v0.3.0`.

### Added

- **Velocidad dedicada en la fila**: etiqueta dedicada a la derecha en cada fila de descargas de la
  interfaz (`Msg::Speed` como nombre accesible), actualizada en el lugar por `sync_row` sin
  reconstruir los widgets de la fila; se oculta automáticamente cuando la velocidad es cero.
- **Descargas programadas por tarea**: `scheduled_at` almacena fecha y hora para diferir el inicio
  de descargas directas y multimedia (migración aditiva 6 sin reescritura de tabla). Formatos
  admitidos: ISO-8601 con zona horaria o `HH:MM` local (hoy, o mañana si la hora ya pasó, resolviendo
  huso horario y DST con `mktime`), normalizados a UTC RFC-3339 (`1004 ERR_INVALID_PARAMS` ante
  formatos no reconocidos). Una tarea futura permanece en `pending` (`start_pending_for_queue`,
  `download.resume` y el relanzamiento multimedia la preservan) y arranca en el tick de 30 s del
  scheduler al cumplirse el momento. En la CLI mediante `--at` y en la GUI mediante el interruptor
  «Programar» del diálogo de alta (calendario + selectores de hora y minuto) con la fila mostrando
  «Programada para …». La ventana diaria recurrente por cola (`queue.schedule`) se mantiene intacta
  y complementaria.
- **Icono en la bandeja del sistema (System tray)**: integración pura en Rust con `ksni` 0.3
  (StatusNotifierItem, sin dependencias C ni librerías de sistema adicionales, preservando CI y
  dependencias del paquete `.deb`). En Ubuntu con la extensión *Ubuntu AppIndicators* activa, el
  clic izquierdo abre el menú, que actúa como el widget de la bandeja: muestra nombre de archivo,
  progreso con porcentaje y bytes descargados/totales (con `—` para tamaños indeterminados en lugar
  de 0%), velocidad y ETA; y acciones «Abrir Segmenta», «Pausar» o «Reanudar» según el estado,
  «Cancelar», «Preferencias» y «Salir de la aplicación» (localizadas en `es`, `en` y `pt_BR`).
  Dispone de su propio cliente IPC en un hilo independiente, suscrito a eventos de tareas
  (`task_progress`, `task_added`, `task_updated`) con refresco limitado a 1 s y sondeo de respaldo
  cada 2 s.
- **Ciclo de vida cerrar a la bandeja**: al estar la bandeja activa, cerrar la ventana principal
  oculta la interfaz y mantiene el proceso vivo (`app.hold()`), preservando las descargas en curso y
  el icono en la bandeja; «Salir de la aplicación» desde el menú de la bandeja termina el proceso.
  Si no hay bandeja disponible (sin `StatusNotifierWatcher`), cerrar la ventana termina el proceso
  como antes. Los comandos de la bandeja cruzan al hilo principal de GTK mediante un canal evaluado
  cada 250 ms.

- **Preferencias → Navegadores sin filas aplastadas**: un botón primario por fila (*Preparar*, o
  *Recargar* cuando hay desincronización) y el resto de las acciones en un menú de tres puntos, en
  lugar de tres botones de texto que dejaban el subtítulo envuelto una letra por línea.
- **Diálogo de descarga coherente**: se eliminó el interruptor *Comenzar ahora* (redundante con no
  programar), *Programar* despliega un selector *Cuándo* con opciones rápidas y el calendario solo
  con *Fecha y hora exacta…*, un resumen indica el momento elegido y ninguna fecha pasada llega al
  daemon: se corrige al instante válido más cercano.
### Fixed
- **Las acciones de la bandeja actúan sobre la descarga viva**: el menú se reconstruía en cada
  refresco y cada ítem capturaba el `task_id` del momento de construcción, así que un clic podía
  pausar una descarga anterior. Las acciones ya no llevan `task_id`: resuelven la tarea activa al
  hacer clic, y los ids de DBusMenu quedan estables entre refrescos.
- **Retomar y Cancelar funcionan con la ventana oculta**: el handler resolvía la tarea contra el
  modelo de la ventana, que deja de refrescarse cuando está en la bandeja; ahora pausa, reanudación y
  cancelación se ejecutan desde el hilo de la bandeja con su propio cliente IPC.
- **El icono aparece cuando el watcher llega tarde**: `StatusNotifierWatcher` pertenece al shell y
  puede faltar al arrancar; en vez de rendirse en el primer intento, el registro se reintenta cada
  5 segundos hasta lograrlo.
- **Velocidad en cero para torrents y streams**: `ProgressPipeline` en `crates/segmenta-daemon/src/manager.rs`
  deriva la tasa a partir de los bytes recibidos entre muestras sobre el intervalo transcurrido y
  calcula el ETA; torrents y transmisiones HLS/DASH, que antes publicaban `speed_bps: 0`, ahora
  reportan su velocidad real.
- **Condición de carrera en sonda de llavero**: `keyring_is_detected_without_the_version_flag`
  reintenta la sonda ante errores transitorios `exec`/`ETXTBSY` en el binario temporal de prueba.
- **Serialización en prueba de apertura de interfaz**:
  `system_open_launches_the_gui_with_the_task_as_an_argument` serializa el acceso a la variable de
  entorno global `SEGMENTA_GUI` con un mutex para evitar colisiones entre tests concurrentes.

## [0.2.4] - 2026-11-15

El ladder de calidades usa los mismos tiers nominales en el daemon y en la extensión, la extensión
deja de reportar errores de localización y de arranque, las descargas se recuperan de un fallo
transitorio, la interfaz puede reintentar una tarea fallida y la extensión informa sin abrir la
ventana. Publicada como `v0.2.4`.

### Fixed

- **Ladder `qualities` por tier**: `media::group_probe` agrupaba por altura exacta, así que un video
  con pistas de `1184/888/592/394/296/196/128` producía un ladder con **sólo audio** y la extensión
  no tenía nada que ofrecer aunque agrupara bien. Ahora `quality_tier` aplica los mismos cortes que
  `protocol.js` (≥1800 → 4K, ≥900 → 1080, ≥600 → 720, ≥300 → 360) y cada entrada conserva la altura
  real y el `format_id` de su mejor candidato, para que `media.add` siga resolviendo con `height<=N`.
- **`default_quality` por tier**: responde `720` cuando ese tier existe aunque su altura real sea
  888, en lugar de comparar distancias de altura.
- **El content script no arrancaba en documentos prerenderizados**: al inyectarse antes de que
  existiera `document.body`, el único listener era `DOMContentLoaded`; si ese evento ya había pasado
  el panel no aparecía nunca y no quedaba ningún error. Ahora arranca con reintento acotado (evento
  `load` + sondeo) e idempotente.
- **Un contexto invalidado ya no rompe la página**: tras recargar la extensión, la pestaña abierta
  conserva un content script huérfano y `chrome.i18n.getMessage` lanzaba «Extension context
  invalidated» en `shared/browser-api.js:176`. `getMessage` devuelve `''` en ese caso, consume el
  `lastError` de una clave ausente y las llamadas al runtime fallan con un código explícito
  (`ERR_CONTEXT_INVALIDATED`) en lugar de un error nativo por llamada.
- **La extensión ya no abre Segmenta al añadir una descarga**: el panel flotante sigue el progreso
  en tiempo real, así que `announceAddedTask` sólo muestra la notificación del sistema con la carpeta
  de destino. La ventana se abre únicamente con el clic explícito en **Abrir Segmenta** / **Ver
  progreso** (`segmenta.open` → `system.open`), sin robarle el foco al navegador.
- **Reintento automático de un stream transitorio**: cuando el host de medios responde 403 a un
  formato concreto (habitual desde IPs de datacenter), `run_download` hace **un** intento más con el
  formato progresivo `18` antes de marcar la tarea como fallida. Cualquier otro fallo, y el segundo
  intento, conservan su propio código.
- **Reintento desde la interfaz**: `download.resume` acepta tareas en `Failed`, limpia
  `error_code`/`error_message` y relanza la tarea multimedia con el plan guardado (URL, tipo, altura
  o formato de audio, destino). Antes respondía `1003` y el botón de reintentar de la GUI no podía
  recuperar una descarga fallida.
- **`contextMenuAddTorrent` faltaba en los catálogos**: el menú contextual de torrents usaba
  `API.getMessage('contextMenuAddTorrent')` sin entrada en `_locales`, así que Chrome registraba un
  `lastError` sin leer («Could not load the message») señalado en `shared/browser-api.js:176`. La
  clave existe ahora en `en`, `es` y `pt_BR`, y el test de locales escanea `background.js` y los tres
  idiomas, que era el hueco que lo dejaba pasar.

## [0.2.3] - 2026-11-15

La extensión vuelve a ofrecer resoluciones de video cuando YouTube reporta alturas que no son las
de la escalera nominal. Publicada como `v0.2.3`. Con esa release, quien venía de 0.2.1 recibe
también el soporte de enlaces cortos y mixes, el menú contextual idempotente y la detección de
versión de extensión de 0.2.2, que no se publicó como versión independiente.

### Fixed

- **Selector de calidad en videos reales**: `groupQualities()` agrupaba por altura exacta
  (`2160/1080/720/360`), así que un video cuyas pistas miden `1184/888/592/394/296/196/128`
  —y cuyo ladder del daemon sólo traía audio— no producía ningún grupo de video y el panel caía
  al fallback de sólo audio. Ahora agrupa por **tier nominal** (>=1800 → 4K, >=900 → 1080,
  >=600 → 720, >=300 → 360) y conserva la altura real y el `format_id` del mejor candidato del
  tier, de modo que `media.add` sigue resolviendo con `height<=N`.
- `defaultQuality()` elige el tier 720 por tier, no por altura exacta: un grupo de 888
  preselecciona 720 en lugar de caer al más cercano.

## [0.2.2] - 2026-11-15

Remediación de la captura de YouTube: enlaces cortos y mixes, un menú contextual que ya no duplica
su id al rearrancar y detección de versión para que una copia vieja de la extensión deje de pasar
desapercibida. Sin tag ni Release publicada.

### Added

- **Enlaces cortos `youtu.be` y mixes**: el panel reconoce la forma corta y las URLs con `list` y
  arma la descarga con `--no-playlist`, así que baja la pista pedida y no la lista entera.
- **Detección de versión de la extensión**: `extension.status` expone `prepared_version`,
  `shipped_version` y `version_sync` (`match`, `mismatch`, `not-prepared`), comparando el
  `manifest.json` de la copia preparada con el del archivo que trae el paquete.
- La página **Navegadores** advierte cuando la extensión preparada no es la enviada, nombra ambas
  versiones y ofrece **Recargar**, que abre la página de extensiones del navegador.

### Fixed

- `Cannot create item with duplicate id segmenta-add-torrent` en Chrome: el menú contextual se
  crea con `removeAll()` antes de cada alta y el callback consume el error de la promesa, así que
  rearrancar el service worker ya no duplica el id ni deja menús muertos.

## [0.2.1] - 2026-11-13

Correcciones de interfaz, captura y motor, y gestión segura de descargas. Publicada como `v0.2.1`.

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
- **iCloud**: el servidor responde `200` con `Content-Range` válido cuando el rango cubre el
  archivo completo; el motor ya lo acepta en lugar de reiniciar hasta el `1028`. Una descarga
  pequeña que termina antes de la pausa de confirmación ya no queda silenciosa: la extensión
  muestra una notificación y permite añadir la URL a Segmenta.
- **Borrado por lote**: el bus silencia los eventos de una tarea mientras se elimina y espera al
  worker; no aparece `state_changed` después de responder. Los tests cubren activas, lote,
  archivos y duplicados.

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
