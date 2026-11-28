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

- [x] T1 — Debrid: contrato de proveedor con metadata y **TorBox** completo (resolución + cuenta), con servidor de prueba local (S1) — ruta: `segmenta-core` — commit: ver L5.
- [x] T2 — Premiumize y Mega-Debrid sobre el mismo contrato (S1) — ruta: `segmenta-core` — commit: `0fd3b01` + ver L6/L7.
- [x] T3 (a) — Device code / OAuth con polling para los proveedores que lo ofrecen, y `segmenta debrid login` (S1) — ruta: `segmenta-core`, `segmenta-cli` — commit: `23cca14`, ver L8.
- [x] T4 — `debrid_accounts` (migración aditiva), prioridad y resolución centralizada en `manager.rs` con `direct: true` (S2) — ruta: `segmenta-daemon` — commit: ver L9.
- [x] T5 — `torrent_via_debrid`: alta remota, polling de progreso, descarga por HTTP y limpieza (S3) — ruta: `segmenta-daemon` — commit: `eb03124` + ver L10/L13.
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

- L5 — T1 cerrado: **TorBox** deja de ser «no soportado» y pasa a tener su flujo real. La API v1 se confirmó contra la documentación oficial antes de escribir una línea: `POST /v1/api/webdl/createwebdownload` y después `GET /v1/api/webdl/requestdl?web_id=…`. Un `success: false` del proveedor se informa como enlace no disponible, no como respuesta rota: el usuario puede actuar sobre eso.
  - Contrato nuevo en `debrid.rs`: `DebridHttp` (lo implementa el daemon en producción y una respuesta canónica en pruebas) y `resolve_with`, que **conduce todos los pasos** del proveedor; Real-Debrid y AllDebrid siguen siendo una sola llamada.
  - Metadata de cuenta para los tres: `account_request` + `parse_account` → `AccountInfo { username, is_premium, expires_at }`. TorBox informa el plan como número (0 es gratis), Real-Debrid `type == "premium"`, AllDebrid `isPremium` + `premiumUntil`. TorBox con `success: false` es `Unauthorized`, no `Malformed`.
  - El token viaja **siempre** en cabecera; hay prueba de que no aparece en la URL del segundo paso.
  - Cinco pruebas nuevas contra respuestas canónicas, incluida la secuencia completa de dos llamadas y el rechazo por enlace no soportado.

- L6 — T2 (a) cerrado: **Premiumize** y el cierre de un defecto latente. Su API se confirmó en la documentación oficial: `POST /api/transfer/directdl` con `src=` y el `Authorization: Bearer` **recomendado por ellos mismos** para que la clave no quede en logs ni en Referer; cuenta por `GET /api/account/info`.
  - **Redacción de credenciales en la URL**: `DebridRequest` gana `credential_in_url` y su `Debug` imprime `…?<redacted>`. Hacía falta antes de sumar proveedores cuyo token viaja en la query por diseño; sin esto, el propio `Debug` era la fuga que la regla de cabeceras intentaba evitar.
  - **Dos fallos distintos, a propósito**: un proveedor que responde bien **sin enlace** es `LinkUnavailable` (el hoster no está disponible para esa cuenta, el usuario puede probar otro); un enlace **inseguro** (`http`, credenciales, espacios) es `Malformed`, porque reintentar no lo arregla.
  - `premium_until = 0` significa «nunca suscripto», no «vencido hace mucho»: no se lee como premium.
  - Cuatro pruebas nuevas (resolución, lista vacía, metadata premium y gratis, y redacción en `Debug`).

- L7 — T2 (b) cerrado: **Mega-Debrid**, el proveedor que no encaja en el molde. Su API no usa un token pegable: se conecta con `login`/`password` (`action=connectUser`) y devuelve un **token de sesión** que viaja en la query (`action=getLink`). Se implementó como flujo de dos pasos sobre `resolve_with`.
  - **Error propio `AccountNotPremium`** (`ERR_DEBRID_NOT_PREMIUM`): `vip_end` significa que la cuenta existe y no puede usar la API. Devolverlo como `Unauthorized` mandaría al usuario a revisar un token que está perfecto.
  - Las credenciales van en el **cuerpo de un POST**, nunca en la query; el paso del token de sesión sí va en la query y por eso la petición se marca `credential_in_url: true`, que ya redacta el `Debug`.
  - `username` queda en `None` a propósito: la API no lo devuelve y no se inventa.
  - Cuatro pruebas nuevas, más un arreglo del arnés: `Canned` ahora **consume respuestas en orden por path**, porque el login y el pedido de enlace comparten `/api.php` y una sola respuesta por path ocultaba el segundo paso.

- L8 — T3 (a) cerrado: **máquina de estados del device flow** para Real-Debrid, con la API confirmada antes de escribirla (`GET /oauth/v2/device/code?client_id=…&new_credentials=yes`, polling por `/device/credentials`). `DeviceFlow` con `device_code`, `user_code`, URL de verificación, intervalo y vencimiento; `PollOutcome::{Pending, SlowDown, Denied, Expired, Granted}`; `next_interval` duplica ante `slow_down` y **capea en 60 s** para que un proveedor no imponga una espera absurda.
  - **Sólo se ofrece donde el flujo existe de verdad.** AllDebrid y TorBox usan API key; Mega-Debrid autentica con la cuenta; Premiumize exige un cliente OAuth registrado. Los cuatro devuelven `Unsupported` con su nombre, en vez de inventar un flujo que fallaría en la primera llamada.
  - **El `client_id` es un parámetro, no una constante**: inventar uno fallaría en el primer request y parecería un bug de integración. Quien llame decide.
  - **Defecto propio corregido por la prueba**: `PollOutcome` derivaba `Debug`, y la variante `Granted` lleva el secreto por usuario. Ahora `Debug` está escrito a mano y redacta. La prueba que lo detectó fue la que exige que el secreto no aparezca al imprimir.
  - El `device_code` viaja en la query → la petición de polling se marca `credential_in_url: true` y el `Debug` la redacta.
  - Falta de T3 para cerrar: intercambio del par por un `access_token` y el comando `segmenta debrid login`.

- L9 — T4 cerrado: **resolución centralizada**. Esquema **v11** con `debrid_accounts(id, provider, label, priority, enabled)`; el token vive en el keyring bajo `debrid:<id>` y la fila sólo dice a quién pertenece. `manager::debrid_resolve` ya **no** describe una petición suelta: recorre las cuentas habilitadas por prioridad y llama a `resolve_with`, que es lo que ejecuta los flujos de dos pasos de TorBox y Mega-Debrid. Antes de este commit, T1 y T2 eran código muerto en el binario.
  - **Fallback real**: `LinkUnavailable`, `QuotaExceeded`, `Unreachable`, `AccountNotPremium` y `Unauthorized` pasan a la cuenta siguiente; se registra cuál se salteó y por qué. Un proveedor que responde fuera de contrato también cae al siguiente, pero queda en el log.
  - **`direct: true`** devuelve el enlace intacto y **no hace ninguna llamada** (hay prueba con el contador del mock en cero).
  - **Sin cuentas, el alias por proveedor sigue funcionando** (compatibilidad con la instalación anterior), y hay prueba de ello.
  - `Manager::with_debrid_http` inyecta la capa HTTP: es lo que hace hermética la prueba de costura. No se finge que `api.real-debrid.com` vive en loopback; se prueba lo que T4 decide —prioridad, caída y bypass— con el `Manager` real, el store real y el keyring de archivo.
  - Cinco pruebas de costura nuevas: prioridad, fallback entre proveedores distintos, `direct`, borrado de cuenta con su token, y compatibilidad con el alias legacy.
  - **Tres pruebas viejas que fijaban el mundo anterior** se actualizaron con su razón: TorBox ya no es «no soportado» (ahora responde 1061 sin token), y dos que comparaban `SCHEMA_VERSION` contra un número pasaron a la constante.

- L10 — T5 (a) cerrado en `segmenta-core/src/debrid_torrent.rs`: modelos del trabajo remoto (`TorrentJob`, `TorrentState::{Queued, Downloading, Ready, Failed, Expired}`, `TorrentFile`) y **tres adaptadores** con sus endpoints verificados contra la documentación oficial: Real-Debrid (`addMagnet`, `info`, `delete`), TorBox (`createtorrent`, `mylist`, `controltorrent`) y Premiumize (`transfer/create`, `transfer/list`, `transfer/delete`).
  - **No es un flujo con un parámetro**: cada proveedor informa distinto. Real-Debrid manda `progress` como porcentaje y `status` como frase; TorBox lo manda como **fracción** dentro de `data`; Premiumize devuelve una lista `transfers`. El parser de cada uno lo trata como su contrato, no como una variante.
  - Un `status` desconocido es `Unsupported` con el nombre del estado, **no** `Malformed` ni uno de los cinco estados adivinado: fingir «descargando» cuando el proveedor dice algo nuevo es peor que decirlo.
  - El token va en cabecera en todos los casos (ninguno de estos endpoints lo necesita en la query) y hay prueba de que el `Debug` no lo imprime.
  - Siete pruebas herméticas con respuestas canónicas: builders de los tres, los tres formatos de identificador, progreso porcentaje vs fracción, archivos seleccionados, estados terminales y las tres llamadas de limpieza.
  - Falta de T5: la orquestación en el daemon (alta, polling con vencimiento, pase de enlaces al motor HTTP y limpieza) y el flag `torrent_via_debrid`.

- L11 — Dos defectos propios corregidos antes de pedir revisión de T5(a): (1) `Box::leak` para devolver el id numérico de TorBox como `&str`, que se ejecutaba **en cada parseo**, o sea en cada poll (`44ffea7`); (2) el estado desconocido de un proveedor se reportaba con `Unsupported(&'static str)`, lo que obligaba a filtrar la cadena o a perder el valor; ahora es `DebridError::UnknownState(String)` con código `1067` (`78570df`). No queda ningún `Box::leak` en el módulo.
- L12 — Estado al cerrar esta sesión: **T1–T4 cerrados y T5(a) cerrado**. Falta de T5 la orquestación en el daemon: alta remota, polling con vencimiento acotado, pase de los enlaces al motor HTTP multi-hilo y limpieza del trabajo remoto **sólo si la descarga local terminó bien**. Después T6–T13 (extracción e integridad, torrents avanzados, RSS) y T14 (cierre con prueba en host). Versión **0.7.0**, sin tag ni release; `main` en `78570df` con los tres gates verdes.

- L13 — T5 cerrado con un límite declarado. `debrid_torrent_job.rs` corre el ciclo completo: alta, polling con `next_interval`, vencimiento de 6 h, cancelación y tope de iteraciones. Con `TorrentState::Ready` encola cada enlace en un `DownloadSink` **inyectable**, y `delete_request` corre **sólo** si el sink aceptó todos los enlaces **y** la finalización local está confirmada.
  - **Lo que encontré al revisar la entrega del worker**: había cableado `|| Ok(())` como callback de finalización local. Es decir, borraba el trabajo remoto **inmediatamente después de encolar**, antes de que se bajara un byte: exactamente el desastre que la condición debía evitar. Un gate que siempre dice "sí" no es un gate.
  - **Arreglado sin fingir**: la producción pasa "no confirmado" hasta que exista seguimiento real de esas tareas, así que el trabajo remoto **se conserva**. El runner devuelve éxito porque los enlaces quedaron encolados —eso sí ocurrió— y registra que la limpieza queda pendiente. Las dos pruebas que esperaban borrado y fallo se ajustaron a la semántica correcta, con la razón escrita.
  - Flag `torrent_via_debrid` (por defecto **off**): con el flag apagado, un magnet sigue yendo a librqbit; hay prueba de que el camino debrid no se toca.
  - Nueve pruebas herméticas: recorrido completo Queued→Downloading→Ready, sink que falla deja el remoto intacto, proveedor vencido corta el polling, `UnknownState` no cuelga, vencimiento por reloj inyectado y el flag apagado.
  - **Pendiente declarado**: confirmar la finalización local de esas descargas para habilitar el borrado remoto. Queda como unidad de seguimiento; no se activa la limpieza destructiva hasta tener esa señal.
