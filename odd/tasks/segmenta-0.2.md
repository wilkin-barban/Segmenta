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
| S2 | `max_active` real por cola y liberación al terminar | `manager.rs`, tests | pendiente |
| A1 | Hook argv seguro (`%file%`, `%folder%`, timeout) | nuevo `postprocess.rs`, `manager.rs`, `store.rs` | pendiente |
| A2 | ClamAV integrado, cuarentena y código `ERR_ANTIVIRUS_FOUND` | `postprocess.rs`, schema v4, RPC, GUI, tests | pendiente |
| L1 | Site logins Basic + keyring y UI | `manager.rs`, `rpc.rs`, `prefs.rs`, tests | pendiente |
| U1 | UI: site logins, antivirus, retirar categorías, respetar `clip_monitor_enabled` | `prefs.rs`, `add_dialog.rs`, `model.rs` | U1a revertida (no compilaba); U1b pendiente de L1/A2 |
| E1 | Extensión manual + GUI; límites documentados | `browsers.rs`, `prefs.rs`, docs | pendiente |
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

