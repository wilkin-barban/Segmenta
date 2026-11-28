# Hito 3 — Debrid, Archivos y Torrents

Estado: planificación; ejecución por unidades con gates propios.
Contrato: `docs/mdm-parity-architecture.md` §3, §5, §6 y §7 (parte RSS).
Versión: **0.7.0**, sin tag ni release hasta cerrar el hito.

## Specs

- **S1 — Multi-Debrid completo.** `segmenta-core/src/debrid.rs` soporta **Real-Debrid, AllDebrid,
  TorBox, Premiumize y Mega-Debrid**; `TorBox` deja de responder `ERR_DEBRID_UNSUPPORTED`. Cada
  proveedor expone **metadata de cuenta** (`username`, `is_premium`, `expires_at`) y, donde el
  servicio lo permite, un flujo **device code / OAuth con polling** que no obliga a pegar tokens a
  mano. Los tokens viven **sólo** en el keyring.
- **S2 — Cuentas ordenables y resolución centralizada.** Tabla `debrid_accounts(id, provider, label,
  priority, enabled)` con el secreto referenciado por alias en el keyring. El daemon resuelve toda
  URL recorriendo las cuentas habilitadas por prioridad, desde **un solo punto** (`manager.rs`), de
  modo que CLI, extensión, Link Grabber y MCP desbloquean igual; `direct: true` saltea el paso.
- **S3 — Torrents remotos vía Debrid.** Con `torrent_via_debrid` activo, un magnet o `.torrent` se
  envía al proveedor, se consulta el progreso remoto y, al completarse en la nube, los archivos se
  descargan con el motor HTTP de Segmenta (multi-hilo, VPN, keyring) y el trabajo remoto se limpia.
- **S4 — Integridad y reparación.** Al finalizar un `.zip`/`.7z`/`.rar` se verifica estructura y CRC
  **sin escribir basura a disco**, hasta un umbral configurable (4 GiB por defecto) o bajo demanda.
  Un ZIP dañado se **repara parcialmente**: se calculan los rangos afectados y se re-descarga sólo
  eso con `Range` + `If-Range`, conservando `ETag`/`Last-Modified` y la tabla de segmentos.
- **S5 — Contraseñas y política de extracción.** Las contraseñas pasan de una variable de entorno a
  una lista **MRU en el keyring**; si ninguna abre el archivo, la extracción se pausa y la GUI pide
  la contraseña (guardándola sólo si funcionó). La política de carpeta es explícita, y con
  `extract_remove_source` el original va a la **Papelera XDG** (`gio trash` /
  `org.freedesktop.FileManager1`), no a un borrado definitivo.
- **S6 — Torrents avanzados.** `torrent.probe` y el alta devuelven un **árbol de archivos** con
  tamaños, carpeta contenedora y detección de colisiones; se descargan y aplican **blocklists**
  (PeerGuardian / eMule `.dat`, plano o `.gz`) con actualización periódica; los **trackers públicos**
  se inyectan sólo a torrents públicos; Segmenta **no pisa archivos que no creó** sin confirmación;
  y hay reproducción mientras descarga (prioridad secuencial + `xdg-open`) y `segmenta torrent create`.
- **S7 — RSS completo.** Un feed que publica `.torrent` o `magnet:` crea tareas BitTorrent filtradas
  como cualquier otro enlace; los feeds se **editan** desde `rss_panel.rs`; y `rss_feeds` persiste
  `last_error` y `last_error_at` para que un feed roto se explique en vez de quedar mudo.
- **S8 — Entrega.** Versión **0.7.0**; commits atómicos de **≤400 líneas**, Conventional Commits,
  **sin `Co-Authored-By` ni atribución a IA**.
- **S9 — Aislamiento y gates.** Pruebas herméticas (sockets, base y certificados en directorios
  temporales; nunca `~/.local/share/segmenta` ni el keyring real). Gates completos antes de cada
  push: `fmt`, `clippy` workspace, `test` workspace con `testserver` y GUI bajo `xvfb-run`.

## Tareas

- [ ] T1 — Debrid: contrato de proveedor con metadata y **TorBox** completo (resolución + cuenta), con servidor de prueba local (S1) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T2 — Premiumize y Mega-Debrid sobre el mismo contrato (S1) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T3 — Device code / OAuth con polling para los proveedores que lo ofrecen, y `segmenta debrid login` (S1) — ruta: `segmenta-core`, `segmenta-cli` — commit: pendiente.
- [ ] T4 — `debrid_accounts` (migración aditiva), prioridad y resolución centralizada en `manager.rs` con `direct: true` (S2) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T5 — `torrent_via_debrid`: alta remota, polling de progreso, descarga por HTTP y limpieza (S3) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T6 — Verificación de integridad al finalizar (zip en Rust, `7z t` para el resto) con umbral y comprobación bajo demanda (S4) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T7 — Reparación parcial de ZIP por `Range` + `If-Range` conservando cabeceras y segmentos (S4) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T8 — Contraseñas MRU en keyring y diálogo interactivo cuando ninguna abre (S5) — ruta: `segmenta-daemon`, `segmenta-gui` — commit: pendiente.
- [ ] T9 — Política de carpeta y envío del origen a la Papelera XDG (S5) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T10 — Árbol de archivos del torrent en probe/alta, con tamaños y colisiones (S6) — ruta: `segmenta-daemon`, `segmenta-gui` — commit: pendiente.
- [ ] T11 — Blocklists P2P y trackers públicos sólo para torrents públicos (S6) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T12 — Propiedad estricta de archivos y reproducción mientras descarga, más `segmenta torrent create` (S6) — ruta: `segmenta-daemon`, `segmenta-gui`, `segmenta-cli` — commit: pendiente.
- [ ] T13 — RSS con `.torrent`/`magnet`, edición de feeds y `last_error`/`last_error_at` (S7) — ruta: `segmenta-daemon`, `segmenta-gui` — commit: pendiente.
- [ ] T14 — Cierre: prueba en host de los binarios release, notas 0.8.0 y gates completos (S8, S9) — ruta: documentación — commit: pendiente.

## Criterios de aceptación

- Los cinco proveedores resuelven un enlace contra su API real o simulada; `TorBox` ya no responde «no soportado».
- La cuenta muestra usuario, si es premium y cuándo expira; ningún token aparece en la base, en un log ni en una respuesta.
- Con dos cuentas del mismo proveedor, la de mayor prioridad se intenta primero y la otra cubre el fallo.
- Un magnet con `torrent_via_debrid` termina descargado por el motor HTTP y el torrent remoto queda limpio.
- Un `.zip` corrupto detecta el daño sin escribir basura, y la reparación re-descarga sólo los rangos dañados.
- Una contraseña correcta extrae y queda en la lista MRU; ninguna correcta pausa y pide, sin guardar basura.
- Con `extract_remove_source`, el original aparece en la Papelera y no desaparece.
- El árbol de archivos del torrent muestra tamaños, permite deseleccionar y avisa cuando un archivo ya existe.
- Un torrent público recibe trackers públicos; uno privado nunca.
- Un feed con `.torrent` crea tareas BitTorrent, el feed se puede editar y un fallo deja `last_error`.
- Versión 0.7.0, árbol limpio y gates en verde al cierre de cada unidad.

## Pruebas previstas

- Debrid: servidor HTTP local que imita cada API (unrestrict, cuenta, device code con polling, errores 401/429); ninguna prueba toca la red real.
- Cuentas: migración aditiva, orden por prioridad, caída al siguiente proveedor cuando uno falla, y token ausente con error que lo nombra.
- Extracción: ZIP generado en la prueba (válido y con un byte alterado), 7z/rar simulados por el runner, contraseña correcta e incorrecta, y Papelera simulada por un comando falso en `PATH`.
- Torrents: `probe` con árbol de archivos, colisión con una tarea existente, blocklist `.dat` y `.gz` de prueba aplicada, y negativa a inyectar trackers en un torrent marcado privado.
- RSS: feed con `<enclosure>` `.torrent` y `magnet:`, edición de un feed persistida, y un 500 del servidor dejando `last_error`.
- Todo con `TMPDIR` propio, socket temporal y `secret-tool` de mentira.

## Riesgos

- **Datos (ítem 1).** La Papelera y la propiedad de archivos son irreversibles si se implementan mal: borrar en vez de mover, o pisar un archivo que el usuario tenía. Van con doble prueba y sin `remove_file` directo.
- **Seguridad (ítem 2).** Tokens de debrid y contraseñas de archivos viven en el keyring; un device code es una credencial temporal y no debe loguearse. Las blocklists bajan de internet: se validan formato y tamaño antes de aplicarlas.
- **Contratos (ítem 3).** `debrid_accounts` es migración aditiva; el alias por proveedor debe seguir funcionando hasta que exista la tabla.
- **Concurrencia (ítem 4).** El polling de debrid y el streaming de torrents agregan estados: sin cancelación explícita quedan tareas huérfanas.
- **Dependencias (ítem 5).** La verificación de ZIP en Rust puede requerir el crate `zip` en el daemon (ya presente) y `flate2` en core (ya presente). Cualquier otra se decide en su unidad.
- **Alcance.** Catorce unidades sobre cuatro áreas: se entregan de a una, con gates y prueba en host, no como un cambio acumulado.

## Log

- L1 — Pedido del usuario (verbatim): «Che, cerramos el Hito 2 de forma impecable en main. Ahora vamos a encarar el Hito 3 (Debrid, Archivos y Torrents) siguiendo docs/mdm-parity-architecture.md (§3, §5, §6 y §7 parte RSS). Antes de tocar código de aplicación, armá el plan ODD en odd/tasks/segmenta-hito3-debrid-archivos-torrents.md con su investigación previa, specs S#, tareas T#, criterios de aceptación y riesgos. El alcance del hito abarca cuatro bloques clave: 1. Multi-Debrid completo en segmenta-core/src/debrid.rs (incorporar TorBox, Premiumize y Mega-Debrid junto a Real-Debrid y AllDebrid), flujo Device Code / OAuth polling donde aplique, metadata de cuenta (username, premium, expiración), tabla debrid_accounts en SQLite con orden de prioridad y tokens en keyring (secrets.rs), resolución centralizada en manager.rs del daemon y descarga de torrents remotos vía Debrid (torrent_via_debrid). 2. Extracción, integridad y reparación en extract.rs: verificación de integridad CRC/estructura al finalizar (.zip, .7z, .rar) sin escribir basura a disco, reparación parcial de ZIPs dañados conservando cabeceras y re-descargando solo los rangos corruptos vía Range HTTP / If-Range, gestor de contraseñas MRU en keyring (secrets.rs) con diálogo interactivo si ninguna abre, y política de extracción con envío del origen a Papelera XDG (gio trash / FileManager1). 3. Torrents avanzados en segmenta-daemon/src/torrent.rs y GUI: árbol de selección de archivos en alta/probe con tamaños y colisiones, descarga y filtrado de blocklists de IPs P2P (PeerGuardian/eMule .dat/.gz) en librqbit, inyección opcional de trackers públicos a torrents públicos, propiedad estricta de archivos en disco (no pisar archivos preexistentes sin confirmación), streaming en GUI con priorización secuencial y acción "Reproducir mientras descarga" (xdg-open), y comando CLI segmenta torrent create. 4. RSS completo para torrents y edición GUI: detección de .torrent y magnet: en feeds RSS creando tareas BitTorrent automáticas filtradas, edición de feeds en rss_panel.rs y persistencia de last_error y last_error_at en rss_feeds. Reglas operativas de siempre: - Mantenemos la versión congelada en 0.7.0 (sin tag ni release todavía). - Todo directo a main con commits atómicos <=400 líneas, Conventional Commits y sin Co-Authored-By ni atribución a IA. - Tests herméticos con sockets/DB temporales (sin tocar ~/.local/share/segmenta ni keyring real del usuario). - Gates completos antes de cada push (fmt, clippy workspace, test workspace con testserver y xvfb-run cargo test -p segmenta-gui). Arrancá investigando el árbol, armá el plan ODD, commitealo y dale luz verde a la primera unidad. Te voy supervisando.»
- L2 — Investigación del árbol: `debrid.rs` (389 líneas) tiene `Provider::{RealDebrid, AllDebrid, TorBox}` con `TorBox` devolviendo `DebridError::Unsupported("torbox")` en resolución y en cuenta; el alias del secreto es `debrid:<provider>` y el seam de resolución vive en `add_dialog.rs`, no en el daemon. `extract.rs` (371) ejecuta `7z`/`unzip` y lee **una** contraseña de `SEGMENTA_EXTRACT_PASSWORD`. `torrent.rs` (332) sondea metainfo con listado de archivos y `torrent_sequential`. `rss.rs` (283) sólo encola HTTP/HTTPS, y `rss_feeds` tiene `last_checked_at` pero **no** `last_error`/`last_error_at`. `rss_panel.rs` (190) no edita feeds.
- L3 — Dependencias disponibles: el daemon ya trae `zip 0.6` con `deflate`, y `segmenta-core` ya tiene `flate2` (de `remotezip`). No hace falta ninguna dependencia nueva para verificar ZIP o leer blocklists `.gz`.
- L4 — Límite de esta entrega: sólo el plan. La primera unidad habilitada es **T1** (TorBox sobre un contrato de proveedor con metadata), que se prueba entera contra un servidor local.
