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
| D1 | OpenSpec 01/03/04/05/07/08/09/10/11 + nuevo 12 | `docs/openspec/` | en curso, partido en D1a1 (01/03), D1a2 (04/05), D1b1 (07/08), D1b2 (09/10/11/12) |
| R1 | Cold-start de captura con reconciliación `downloads.search` | `extension/*/background.js`, tests | implementado (`dfb1b2f`); falta QA real de cold start y alinear 03 §5.4 |
| R2 | Probe multiarchivo de torrents | `crates/segmenta-daemon/src/torrent.rs`, tests | hecho (`d7075a6`) |
| R3 | `extension` visible en `--help` y docs CLI | `crates/segmenta-cli/src/cli.rs`, `render.rs` | hecho (`9024726`) |
| S1 | Scheduler de colas nativo y ventanas horarias | nuevo `scheduler.rs`, `manager.rs`, `main.rs`, `store.rs` | pendiente |
| S2 | `max_active` real por cola y liberación al terminar | `manager.rs`, tests | pendiente |
| A1 | Hook argv seguro (`%file%`, `%folder%`, timeout) | nuevo `postprocess.rs`, `manager.rs`, `store.rs` | pendiente |
| A2 | ClamAV integrado, cuarentena y código `ERR_ANTIVIRUS_FOUND` | `postprocess.rs`, schema v4, RPC, GUI, tests | pendiente |
| L1 | Site logins Basic + keyring y UI | `manager.rs`, `rpc.rs`, `prefs.rs`, tests | pendiente |
| U1 | UI: site logins, antivirus, retirar categorías, respetar `clip_monitor_enabled` | `prefs.rs`, `add_dialog.rs`, `model.rs` | pendiente |
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

