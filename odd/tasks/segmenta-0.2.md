# Segmenta 0.2.0 — scheduler, antivirus, site logins, hardening y documentación

Plan aprobado: `/home/wilkin/.commandcode/plans/segmenta-0.2.md`.
Sin tag ni Release hasta que el QA real en Ubuntu y la matriz de pulido estén verdes.

## Decisiones cerradas

- Scheduler nativo en el daemon; `max_active` real por cola.
- Hook argv seguro + ClamAV integrado; infección → cuarentena + tarea fallida.
- Site logins Basic + GNOME Keyring; sin Digest.
- Sin categorías automáticas; retirar de UI/spec, conservar lectura legacy.
- Sniffer sólo YouTube + captura genérica; sin adaptadores nuevos.
- Extensión manual asistida por GUI; sin tiendas.
- Versión 0.2.0; tag `v0.2.0` sólo al final.

## Definition of Done (pulido)

Ninguna unidad se marca `done` si queda:

- warning de `cargo fmt` o `clippy -D warnings`;
- test Rust/Node fallido o skip silencioso por display/dependencia;
- `sync --check`, `pack --check` o `check-package.sh` distinto de verde;
- warning accionable de desktop/appstream/lintian;
- error o warning nuevo en daemon/GUI/extensión durante QA real;
- hallazgo abierto de captura, scheduler, ClamAV, auth, torrents o docs;
- contradicción README/CHANGELOG/OpenSpec/QA con el comportamiento;
- proceso, audio, fixture o tarea de prueba residual en Ubuntu.

Ciclo: agente implementa → yo verifico y hago QA real → error exacto vuelve al agente → corrige y repite gates → sólo entonces `done`.

## Tasks

| ID | Entrega | Archivos principales | Estado |
| --- | --- | --- | --- |
| D0 | Documentación P0: README 0.1.2/0.2, CHANGELOG 0.1.0–0.1.2, QA 0.1.2, release notes 0.2.0 | `README.md`, `CHANGELOG.md`, `docs/qa/`, `packaging/release-notes-0.2.0.md` | hecho (`cfa702f`) |
| D1 | OpenSpec 01/03/04/05/07/08/09/10/11 + nuevo 12 | `docs/openspec/` | hecho: `2a2e42a`, `95e045a`, `ccf74c7`, `95364f5`, `df5733b`, 07/08 reparados |
| R1 | Cold-start de captura con reconciliación `downloads.search` | `extension/*/background.js`, tests | implementado (`dfb1b2f`); falta QA real de cold start y alinear 03 §5.4 |
| R2 | Probe multiarchivo de torrents | `crates/segmenta-daemon/src/torrent.rs`, tests | hecho (`d7075a6`) |
| R3 | `extension` visible en `--help` y docs CLI | `crates/segmenta-cli/src/cli.rs`, `render.rs` | hecho (`9024726`) |
| S1 | Scheduler de colas nativo y ventanas horarias | nuevo `scheduler.rs`, `manager.rs`, `main.rs`, `store.rs` | hecho (`8099ee2`) |
| S2 | `max_active` real por cola y liberación al terminar | `manager.rs`, tests | hecho (`3c0d9fa`) |
| A1 | Hook argv seguro (`%file%`, `%folder%`, timeout) | nuevo `postprocess.rs`, `manager.rs`, `store.rs` | hecho (`015942e`) |
| A2 | ClamAV integrado, cuarentena y código `ERR_ANTIVIRUS_FOUND` | `postprocess.rs`, schema v4, RPC, tests | daemon hecho (`d20890b`, corregido en `6475b27`); vista de cuarentena en `f9c3980` |
| L1 | Site logins Basic + keyring y UI | `manager.rs`, `rpc.rs`, `store.rs`, `schema.sql`, `prefs.rs` | hecho: backend `1e3f4e2`/`6da910d`, página `d19ca14`/`56c126f`, carga al abrir `1cf99cf` |
| U1 | UI: site logins, antivirus, retirar categorías, respetar `clip_monitor_enabled` | `prefs.rs`, `add_dialog.rs`, `model.rs` | U1a1 `c6b735a`, U1a2 `b8f2732` y antivirus `f9c3980` hechos; falta la página de site logins (depende de L1) |
| E1 | Extensión manual + GUI; límites documentados | `browsers.rs`, `prefs.rs`, docs | hecho (`0d92384`) |
| Q1 | Gates completos, `.deb` 0.2.0, QA Ubuntu y artefactos | `packaging/`, `dist/`, QA docs | pendiente |
| REL | Tag/Release 0.2.0 sólo con Q1 verde | git/GitHub | pendiente |

## Gates por unidad

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node extension/tests/run.mjs
node extension/tools/sync-shared.mjs --check
node extension/tools/pack.mjs --check
packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0
```

Para GUI: `xvfb-run -a cargo test -p segmenta-gui`; no aceptar skips por falta de display.

## QA real Ubuntu

- Instalar `.deb` 0.2.0 y arrancar por systemd.
- `extension.status/prepare/open` en Chrome for Testing y Firefox oficial.
- YouTube: 4K/1080p/720p/360p + audio, 720p por defecto; handoff arriba.
- Captura `.zip`: confirmación nombre/host/carpeta, aceptar/rechazar/timeout/host caído.
- Cold-start: primera descarga al arrancar Chrome.
- Scheduler: ventana corta y `max_active`; nunca probar suspender/apagar la máquina.
- ClamAV: fixture EICAR, cuarentena, tarea fallida, sin archivo en Descargas.
- Site login Basic contra servidor local; no filtrar secreto en logs.
- Torrent multiarchivo: total y files correctos.
- CLI: `segmenta --help` incluye `extension`.
- Limpieza: procesos, audio, fixtures, tareas, `allow_local_downloads=false`.

## Reglas de trabajo

- Rama `main`, commits pequeños y gates por unidad.
- No tag, no Release, no push de release hasta aprobación final.
- No tocar el tag `v0.1.2` ni sus assets.
- No inventar claves de revisión RDD; la revisión no autoriza entrega.
- Reportar commits, SHAs, gates, hallazgos y bloqueos.

## Log

- L1: D0 cerrado en `cfa702f`. README deja de afirmar alcance 0.1.0, instala el `.deb` que construye el repo y declara **0.2.0 en preparación**; CHANGELOG gana `[0.1.0]`–`[0.1.2]` con compare links y conserva `[Unreleased]` como placeholder; `docs/qa/segmenta-0.1.2-live.md` registra el QA real de Ubuntu (instalación 0.1.2-1, `native-ready` con `extension_installed:false`, `prepare` chromium/firefox-0.1.2, panel 4K/1080p/720p/360p/audio con 720p, handoff, captura aceptar 524288/524288 y rechazar sin tarea) y marca como no reproducido sólo Firefox 0.1.2 y la suspensión real del service worker MV3; `packaging/release-notes-0.2.0.md` creado con los límites del plan. Corregida de paso la afirmación falsa de `release-notes-0.1.2.md` («instalada y conectada»): los estados reales son `native-ready`, `native-missing`, `extension-installed` y `unsupported`.
- L2: D1 se partió porque dos workers de documentación agotaron ~7 M de contexto sin devolver. Reparto definitivo con superficies disjuntas: D1a1 (01/03), D1a2 (04/05), D1b1 (07/08), D1b2 (09/10/11 + nuevo 12). Regla operativa nueva: ningún worker pasa de ~2 M sin devolver; la documentación se entrega por pares de archivos.

## Deuda GUI confirmada (insumo de U1)

Verificada contra el código durante D1a2; ninguna de estas contradicciones es un bug de documentación, son del código:

1. `crates/segmenta-gui/src/prefs.rs:58-77` todavía pinta el `AdwExpanderRow` legacy de categorías: el contrato de 0.2 dice que no hay categorías en la UI. Retirar la fila y conservar sólo la lectura legacy del ajuste.
2. Literales que ya tienen entrada en `segmenta-i18n` y no la usan: `prefs.rs:80` («Sesión»), `prefs.rs:119`/`124` («Modo»/«Servidor»), `prefs.rs:240` («Versión»), `window.rs:1188`/`1215` (notificaciones: debe usarse `FailedNotification`/`CompletedNotification`, cuya cadena real de completada es «Descarga terminada», no «completada»).
3. `add_dialog.rs` prellena desde el portapapeles sin consultar `clip_monitor_enabled`: elegir una semántica y documentarla.
4. `model.rs::DownloadStatus` no conoce `retrying` ni `finalizing`: se pintan como «Desconocida» y sin acciones.
5. `window.rs:1410-1458` deriva las colas de `download.list` en lugar de `queue.list`, y limita el snapshot a 500 tareas.
- L3: D1a1 se entregó sin worker: el tercer intento volvió a inflarse (7.2 M en 8 min) y se canceló. Inspeccioné el borrador, lo contrasté con el código y con R1 §5.4, corregí la incoherencia real (el documento decía que la reconciliación de cold-start seguía «en preparación» cuando `dfb1b2f` ya la implementa con `MAX_RECONCILE_DOWNLOADS = 20` y `RECONCILE_THROTTLE_MS = 5 min`) y commiteé 01 y 03 por separado. Regla: por encima de ~2 M de contexto el worker de documentación se corta y la unidad se termina en línea.
- L4: D1 completo. 07 quedó con el conjunto cerrado de 27 métodos despachados, `torrent.add` descrito como es tras R2 (`files_count = 1`, una fila `torrent_files`), `batch_id` en `media.add`, el mapeo real de `queue.remove` a 1001 y la allowlist como array literal. 08 cita `schema.sql` byte a byte, separa los deltas v2/v3 en §4.2.1 y las columnas v4 en §4.2.2, y corrige la fila v1 y el `old_state` de `recover_crashed`. Hallazgos de código anotados, no maquillados: el comentario de `proxy.rs` nombraba cuatro prefijos y el array tiene cinco (corregido en `ced0664`), y `torrent.add` sigue ignorando `files[]` del probe R2 → nueva tarea R2b, a ejecutar cuando S1/S2 liberen `manager.rs`.
- L5: S1 entregado en `8099ee2` (8 tests propios, `clippy` y `fmt` limpios, 293 tests Rust). Scheduler tokio con tick de 30 s, ventanas que cruzan medianoche, reloj inyectable, arranque de pendientes por una sola entrada compartida con `resume`, `action_on_complete` sólo al vaciarse la cola y parada por el canal de apagado del daemon.
- L6: **U1a revertida**. El worker murió a mitad de la unidad y dejó la GUI sin compilar: `cargo test -p segmenta-gui --test window` fallaba con `E0599` en cinco puntos — `DemoClient::set_queues`, `SegmentaWindow::queue_names` (×2) y `SegmentaWindow::truncation_notice` (×2). Esas funciones eran justamente la parte 5 de la deuda (colas desde `queue.list` y aviso de truncado). No commiteé código que no compila; revertí `gui/src/{model,prefs,window}.rs`, `gui/tests/{model,window}.rs` y `i18n/src/lib.rs`, borré el `probe_red.rs` temporal, y la deuda de GUI vuelve a estar entera pendiente. La unidad se rehará partida: primero los puntos 1–4 (categorías, literales, `clip_monitor_enabled`, estados), después el 5 (colas y truncado) que es el que necesita tocar `client.rs` y añadir las funciones que faltaban.
- L7: S2 entregado. `free_slots` lee el límite de la fila de la cola en cada llamada (nunca cacheado) y `start_pending_for_queue` arranca sólo lo que cabe bajo un `start_lock`, de modo que dos llamadores no puedan tomar el mismo slot; el mutex `running` no se sostiene a través de awaits. Tests: 15 en `--test scheduler` (8 de S1 + 7 de S2) y 300 en el workspace, `fmt` y `clippy` limpios.
- L8 (límite documentado, no un bug abierto): la **revisión nativa RDD no es utilizable en este repositorio** con el presupuesto actual del proveedor. El último intento real —candidato de dos archivos y 348 líneas, base `d621520`— detuvo `review.start` en preflight con `lens_context_budget_exceeded`, sin crear autoridad, e indicó que reintentar el mismo candidato no puede funcionar. Los dos linajes previos se cerraron de forma auditada (uno abandonado por un `subject-hash` truncado del proveedor, otro escalado por causalidad desconocida y luego abandonado). Consecuencia asumida: la garantía de calidad de 0.2.0 descansa en los gates por unidad, en la verificación independiente cuando aporta y en el QA real del usuario, y así queda escrito. No se inventan claves de revisión ni se presenta la ausencia de revisión como aprobación.
- L9: A1 entregado en `015942e`. Hook argv puro con `%file%`/`%folder%`/`%name%` expandidos como valor completo de un argumento y en una sola pasada, `stdin` en `/dev/null`, timeout propio y salida acotada; el código de salida y el mensaje se registran sin convertir una descarga correcta en fallida. Sin cambio de esquema en esta unidad (verificado: `schema.sql` intacto), como pedía la frontera con A2. 11 tests propios; 313 Rust en total.
- L10: U1a1 entregado en `c6b735a` tras la reversión de la v1. Fuera el expander legacy de categorías (el ajuste sólo se lee), cuatro literales pasan al catálogo con la cadena real «Descarga terminada», el prellenado respeta `clip_monitor_enabled` y `retrying`/`finalizing` se pintan como sí mismos con sus acciones. El punto 5 de la deuda (colas desde `queue.list` y aviso de truncado) queda como U1a2, que es donde la intentona anterior dejó cinco `E0599`.
- L11: A2 (parte daemon) entregado en `d20890b`. Schema v4 con `quarantine_path`/`postprocess_exit`/`postprocess_message` y migración idempotente que conserva las filas v3; settings `antivirus_*` validados; detección `clamdscan` → `clamscan` con `SEGMENTA_ANTIVIRUS_PATH` como override; infección → cuarentena, tarea `failed` con 1046 y archivo fuera de Descargas; motor ausente → 1047, nunca `completed`. Tuve que completar la unidad en línea: el worker dejó el test sin compilar (faltaban `Duration`, `Instant`, `PermissionsExt` y `PathBuf`) y **sin prueba de cuarentena**, que era el corazón del pedido; añadí dos tests —el archivo sale de Descargas y llega íntegro a cuarentena, y dos infecciones homónimas no se pisan— y quedaron 15 en `--test antivirus`.
- L12: contrato actualizado a 1047 en `07` y `12`, con la razón: sin número la GUI tendría que adivinar por el texto y el plan pide código estable.
- L13: U1a2 entregado en `b8f2732`. Las colas salen de `queue.list`, una lista vacía no inventa colas y el snapshot recortado en 500 muestra aviso con el total real. Con esto la deuda de GUI confirmada queda saldada salvo las páginas de site logins y antivirus, que son U1b.
- L14 (bloqueo real, no burocracia): L1 se implementó y **se revirtió sin commit** porque su propia prueba demostró una fuga de credenciales. Con `site_login.set` para `127.0.0.1` y una tarea cuya URL redirige a otro puerto del mismo host, el servidor de destino recibió `Authorization: Basic YW5hOnMzY3IzdA==`. La prueba que lo detecta es `credentials_do_not_follow_a_cross_origin_redirect` y su rojo exacto fue: `a cross-origin redirect must not carry the credentials: ["Basic YW5hOnMzY3IzdA=="]`. Diagnóstico parcial ya hecho: `final_url` no se actualiza nunca en `manager.rs` (sólo se escribe `None` al crear), así que comparar `final_url` con `url` no protege nada, y el login se indexa por *host* (sin puerto), por lo que un salto de puerto sigue encontrando la credencial. La otra prueba que fallaba, `listing_never_leaks_the_password`, era un defecto de la prueba: su `!raw.contains("password")` chocaba con la clave legítima `has_password`. Consecuencia: L1 no entrega código a medias; queda como unidad enfocada con esta evidencia, y su UI (prefs) depende de ella.
- L15 (decisión del usuario, cierra L14): los site logins se indexan por **origen exacto** (`scheme` + `host` + `puerto`), no por host. `Authorization: Basic` se inyecta sólo para el origen de la URL original/probe; tras un redirect no se reinyecta ni se re-resuelven credenciales para el origen nuevo, y si cambia el scheme, el host o el puerto la cabecera se descarta (la regla cross-origin existente se mantiene). Requiere migración para el campo `origin` sin romper filas legacy, y la UI debe mostrar el origen. Política de compatibilidad elegida: una fila legacy sin `origin` se conserva y se lista, pero **no se usa para inyectar** hasta que el usuario fije su origen; deducir un scheme por defecto sería mandar credenciales a un origen adivinado. Tests exigidos: mismo host con otro puerto no recibe Basic, redirect externo no recibe Basic, origen exacto sí. Se ejecuta en cuanto R2b libere `manager.rs`/`store.rs`.
- L16: R2b entregado. `torrent.add` toma el `files[]` del probe (o el que ya trae el add), valida cada ruta y persiste una fila por archivo con índices contiguos, longitud y `selected: true`, con `files_count` real; el single-file conserva una fila y un probe sin archivos falla el add en vez de inventar el archivo 0. 13 tests en `--test torrent`, 339 Rust en el workspace, `fmt` y `clippy` limpios.
- L17: U1b-antivirus entregado en `f9c3980`, terminado en línea porque el worker dejó otra vez los tests sin el código (`antivirus_page`, `detail_state`, `detail_quarantine`, `detail_path`, `ERR_ANTIVIRUS_UNAVAILABLE`). Implementé la página con el interruptor, el timeout con sus límites reales 1–3600 y la carpeta de cuarentena, más el estado propio «En cuarentena» con su ruta y la supresión de «Abrir carpeta» para un archivo que el daemon ya movió. 1046 y 1047 quedan separados: motor ausente no se pinta como infección. 14 tests en `--test window`, 343 Rust en el workspace.
- L18: E1 entregado en `0d92384`: Navegadores explica el flujo manual con las tres acciones reales y un instructivo numerado, y los tres estados (no soportado, sólo manifiesto, extensión instalada) pintan textos distintos. De paso, mi cambio de schema v5 rompió dos aserciones de A2 que fijaban v4: ahora comparan contra `SCHEMA_VERSION` en vez de un número congelado (`50e9cee`). Gates: 354 tests Rust, 51 suites, GUI bajo xvfb sin fallos, `fmt` y `clippy` limpios.
- L19 (estado de L1 al cerrar esta ventana): pasos 1–2 hechos (`1e3f4e2`): schema v5 con `origin`, store por origen exacto, fila legacy listada y jamás usada para inyectar, y el caso de migración sin tabla cubierto. Faltan el paso 3 (RPC `site_login.list|set|remove` con `normalize_origin`), el paso 4 (inyección por el origen real de la petición, no por `final_url`, que hoy nunca se actualiza) y el paso 5 (tests del leak: mismo host otro puerto, redirect externo, origen exacto). No se deja como bug abierto: es una unidad a medio camino con el modelo ya decidido y probado.
- L20: L1 backend cerrado en `6da910d`. `site_login.list|set|remove` por origen exacto (`scheme://host[:port]`, puerto por defecto omitido), contraseña sólo al llavero con alias `site/<origin>`, `list` sin clave `password`, rechazo de orígenes con ruta, credenciales embebidas o esquema distinto de http/https. Inyección resuelta por el **origen real de la petición**, no por `final_url` (que nunca se actualiza): otro puerto, otro esquema u otro host no reciben nada, y una fila legacy sin origen se lista pero no inyecta. Base64 implementado en el crate (diez líneas) para no sumar una dependencia al `.deb`. RED observado: con el puerto eliminado de la clave de origen, `the_exact_origin_gets_basic_auth_and_no_neighbour_does` falla con «a different port must not receive the credentials»; con la clave correcta, 5/5 verdes. Gates: 359 Rust, `fmt` y `clippy` limpios.
- L21: U1b-site-logins entregado en `d19ca14` y montado en el diálogo en `56c126f`. La página lista un origen por fila con su usuario, avisa del estado vacío y de una fila legacy sin origen, guarda por `site_login.set` con el origen tipeado y **limpia el widget de contraseña al enviarlo**, y borra por el origen de su propia fila. La contraseña es write-only: no vuelve del daemon y no queda en la UI. Todo el texto en es/en/pt. Tests GUI bajo xvfb (5 casos en un solo test porque GTK exige un único hilo inicializador): orígenes visibles, sin contraseña en la lista, estado vacío, guardado completo, formulario incompleto que no llega al daemon y borrado con su origen. Pendiente declarado: la página se construye con lista vacía y no carga `site_login.list` al abrir el diálogo; las altas y bajas sí funcionan. Gates: 359 Rust, GUI sin fallos, `fmt` y `clippy` limpios.
- L22: L21 cerrado en `1cf99cf`. Al abrir la página se pide `site_login.list` en un hilo de trabajo y se repinta desde el bucle principal con un timeout que se rinde si el widget desapareció; si la llamada falla se deja el estado vacío en vez de fingir que la lista es autoritativa. El `DemoClient` ahora responde `site_login.list|set|remove` para poder probarlo, y el test verifica que un login preexistente aparece como origen en la lista y que no hay contraseña en ningún sitio. Gates: GUI bajo xvfb sin fallos, `cargo test --workspace` sin fallos, `fmt` y `clippy -D warnings` limpios.
- L23 (bloqueante del QA real, corregido en `6475b27`): con `antivirus_enabled=true`, sin `postprocess_command` y un EICAR servido en local, la tarea terminaba `completed` con `error_code: null` y el archivo intacto. Causa exacta: `run_postprocess` hacía `return` cuando el comando estaba vacío, así que nunca llegaba al escaneo, y `finish_task` marcaba `Completed` sin mirar el resultado. Arreglo: `run_postprocess` devuelve `FinalizeOutcome` (Clean / Infected / ScanUnavailable / ScanFailed), el hook es opcional, el escaneo corre siempre con antivirus activo sobre el archivo que dejó el hook, y `finish_task` decide: limpio → `completed`; infectado → cuarentena, `failed` con 1046 y `quarantine_path`; motor ausente o escaneo sin veredicto → `failed` con 1047; nunca `completed` antes del escaneo. Tres tests de integración con `clamscan` falso y EICAR real reproducen el escenario: infectado (1046, archivo fuera de Descargas), limpio (completed) y sin motor (1047). Los tests serializan el override `SEGMENTA_ANTIVIRUS_PATH` con un mutex async porque es estado de proceso. Gates: 0 suites fallidas, `clippy -D warnings` limpio.

