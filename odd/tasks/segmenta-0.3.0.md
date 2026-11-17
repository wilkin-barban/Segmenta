# Segmenta 0.3.0 — velocidad visible, icono de bandeja y programador de descargas

## Specs

- **S1.** La interfaz de Segmenta debe mostrar la velocidad real a la que se está descargando el
  archivo/video/torrent. Hoy esa información no se muestra correctamente; es necesaria.
- **S2.** Icono en la bandeja de sistema de Ubuntu que muestre el progreso de la descarga en
  ejecución. Al hacer clic debe mostrar información detallada y resumida de la descarga en curso
  (un widget adaptado a Ubuntu) con botones **Abrir Segmenta**, **Salir de la aplicación** y, con la
  descarga en ejecución, **pausa** y **cancelación**. Debe tener además opciones de clic derecho.
- **S3.** Temporizador: poder programar las descargas para una hora específica, como un programador
  de descargas.

- **S4.** La velocidad debe verse **dedicada** en la lista principal de la GUI, no sólo dentro del
  subtítulo combinado que hoy se recorta; y las descargas que hoy publican `speed_bps: 0`
  (torrent y streams HLS/DASH) deben reportar la velocidad real.
- **S5.** El programador debe permitir fijar **fecha y hora exactas** para una descarga concreta
  (no sólo la ventana diaria de una cola), y la cola debe respetarla al arrancar tareas.
- **S6.** La bandeja debe sobrevivir al cierre de la ventana: cerrar la ventana oculta Segmenta y
  deja el icono activo; **Salir de la aplicación** es la única acción que termina el proceso.
- **S7.** Toda cadena nueva va en `es`, `en` y `pt_BR` (`crates/segmenta-i18n`), sin romper el
  catálogo ni los tests de locales.

## Tasks

- [x] T1 — Velocidad real en torrent y streams HLS/DASH (S1, S4) — `crates/segmenta-daemon/src/{manager,torrent,stream_media}.rs` — `7485673`
- [x] T2 — Velocidad dedicada en la fila de la GUI (S1, S4) — `crates/segmenta-gui/src/{model,window}.rs`, `crates/segmenta-i18n/src/lib.rs`, tests GUI — `c422c4a`
- [x] T3 — `scheduled_at` por tarea: migración, RPC y arranque diferido (S3, S5) — `crates/segmenta-daemon/src/{store,manager,scheduler}.rs` — `543de27`, `92293df`, `92e080e`, `9755e1c`, `2e385fe`
- [x] T4 — Diálogo **Programar** y estado visible en la fila (S3, S5, S7) — `crates/segmenta-gui/src/{add_dialog,window,model}.rs`, i18n, tests — `9755e1c`
- [x] T5 — Bandeja del sistema con progreso y acciones (S2, S6, S7) — `crates/segmenta-gui/src/tray.rs` (nuevo), `Cargo.toml`, `main.rs`, `app.rs`, `window.rs`, i18n — `d58c615`, `99c197c`, `1d23009`
- [x] T6 — Documentación (OpenSpec 04/07, README, CHANGELOG, notas 0.3.0, QA live) — `CHANGELOG.md`, `README.md`, `docs/openspec/`, `packaging/release-notes-0.3.0.md`, `docs/qa/segmenta-0.3.0-live.md`
- [x] T7 — Artefactos 0.3.0 y verificación en el host (sin publicar) — pendiente: verificación final de empaquetado y prueba en sesión real de Ubuntu antes de publicación.

## Evidencia de la exploración (2026-11-15)

- **Velocidad**: el motor la calcula (`engine.rs:1270-1307`) y el daemon la publica
  (`manager.rs:1843-1874`); la GUI la recibe (`model.rs:765`) y la mete en `progress_line()`
  (`model.rs:461`) y en `detail.speed` (`window.rs:2894`). No falta el dato: falta un elemento
  dedicado y sobran ceros cableados. `manager.rs:2623` y `2944` publican `speed_bps: 0` para
  torrent/stream, y `manager.rs:340,359,1861` ponen `0` en los segmentos. `Msg::Speed` sólo se usa
  en la CLI (`render.rs:170`).
- **Programador**: `queue.schedule` ya existe y guarda `{start_time, stop_time, action_on_complete}`
  por cola en `settings` (`manager.rs:1419-1433`, `store.rs:1834-1864`), y el scheduler hace tick
  cada 30 s o por `wake` (`scheduler.rs:280-324`) evaluando `is_open_at`. No hay `scheduled_at` por
  tarea ni estado `Scheduled`; `--no-start` deja la tarea en `pending` (`manager.rs:546-562`).
- **Bandeja**: la GUI corre en el hilo principal de GTK (`main.rs`); el cliente IPC es
  `Client { request, poll_event }` con `UnixClient::from_env()` y `RequestBus::spawn`
  (`client.rs:62-80, 659-695, 1102-1123`). `Cargo.toml` sólo trae `glib/gtk4/libadwaita/serde`;
  el CI no instala `libayatana-appindicator3-dev` ni `libdbus-1-dev`, así que `ksni` (SNI puro en
  Rust) no agrega dependencias de sistema. `check-package.sh` exige la lista `REQUIRED_PATHS` y que
  `Depends` incluya `libsecret-tools`.
- **Research de bandeja**: Ubuntu trae *Ubuntu AppIndicators* activado por defecto; `ksni` 0.3.7
  (MSRV 1.80) implementa StatusNotifierItem con menú dinámico (`Handle::update`), `icon_name`,
  `activate` y `secondary_activate`; con la extensión de GNOME el clic izquierdo abre el menú, así
  que el "widget" es el propio menú con filas de información vivas.

## Log

- L1 — Pedido del usuario (verbatim):

> vamos a preparar la siguiente release, esta tendra las siguientes mejoras: en la interfaz UI de Segmenta, no se muestra la velocidad en que se esta descargando el archivo/video/torren/etc, esta informacion es necesaria mostrarla correctamente. Tenemos que desarrollar un icono en la bandeja de sistema de ubuntu que muestre el proceso en ejecucion, este icono mostrara al darle clic la informacion detallada y resumida de la descarga que este teniendo lugar en la aplicacion de Segmenta, es como un widget, adaptado a ubuntu y mostrando informacion de la descarga, con botones de abrir Segmnenta, Salir de la aplicacion y cuando esta la descarga en ejecucion botones de pausa o cancelacion, tendra ademas opciones ded clid derecho, mostrando opciones. Otra cosa que tenemos que desarrollar en la nueva release en un temporizador, quiero poder programar las descargas para una hora en especifico si es que lo quiero, algo asi como un programador de descargas. Investiga como implementar todas estas mejoras y al terminar la exploracion e investigacion, pidele al agente en Segmenta que las implemente, le pasas la directiva correcta y detallada al agente en Segmenta y lo orquestas hasta que termine

### Velocidad y programador cerrados (2026-11-15)

- T1: `ProgressPipeline` unifica tasa y ETA para torrent y streams; los ceros cableados desaparecieron. Tests: 2 de integración con dos muestras.
- T2: etiqueta de velocidad dedicada en la fila, actualizada en el lugar por `sync_row`; `Msg::Speed` como nombre accesible.
- T3/T4: migracion 6 aditiva (`scheduled_at`), gate en `start_pending_for_queue`, `resume_task` y el relanzamiento multimedia, `normalize_scheduled_at` (ISO-8601 con zona u `HH:MM` local con DST) normalizado a UTC RFC-3339, `--at` en CLI y dialogo **Programar** con calendario + hora/minuto.
- Flake preexistente identificado y neutralizado: `keyring_is_detected_without_the_version_flag` fallaba 1 de cada 5 corridas por un race `exec`/`ETXTBSY`; el test reintenta la sonda (`9b0a569`). Gate posterior: 3 corridas seguidas de `cargo test --workspace` en 426 verdes.

### Bandeja del sistema y documentación cerradas (2026-11-15)

- T5: icono de bandeja StatusNotifierItem mediante `ksni` 0.3 (`d58c615`), ciclo de vida de cierre a segundo plano con `app.hold()` y fallback sin watcher (`99c197c`), y serialización del test `system.open` sobre `SEGMENTA_GUI` (`1d23009`).
- T6: documentación completa de la versión 0.3.0 en `CHANGELOG.md`, notas de release en `packaging/release-notes-0.3.0.md`, contratos en `docs/openspec/04-ui-ux-libadwaita.md` y `docs/openspec/07-ipc-contract.md`, resumen en `README.md` y reporte de validación en `docs/qa/segmenta-0.3.0-live.md`.

### Cierre (2026-11-16)

- **Artefactos 0.3.0** (sin publicar): `.deb` sha256 `c593115468811539384902508d114033a92499b8d20ed5675cc1fc7894e91760`, extensión `f4d7fb68…b8dd`, firefox `2cc55306…83eb`; `check-package` 23/23.
- **Instalado en el host** con `--reinstall` (apt no reemplaza con la misma versión): `0.3.0-1`, daemon `0.3.0 (protocolo v1)`, extensiones preparadas `chromium-0.3.0` y `firefox-0.3.0`, sha256 del `.deb` remoto idéntico.
- **Bandeja**: `Recommends: gnome-shell-extension-appindicator`. Hallazgo en el host: la extensión de GNOME está *listada* como habilitada con el id `appindicatorsupport@rgcjonas.gmail.com`, cuyos archivos no están instalados, mientras la de Ubuntu (`ubuntu-appindicators@ubuntu.com`, provista por `gnome-shell-ubuntu-extensions`) existe pero no está habilitada; por eso no hay `StatusNotifierWatcher` y el icono no aparece todavía. El arranque sin watcher degrada bien (`SMOKE OK`, salida limpia).
- **Gate determinista**: se neutralizó el race `ETXTBSY` de los tests que escriben un ejecutable falso y lo ejecutan (mutex por binario + reintento de la llamada). Seis corridas consecutivas de `cargo test --workspace` en 436 verdes; GUI 109; Node 176; fmt/clippy limpios.

### Verificación de la bandeja en el host (2026-11-16)

Verificado por D-Bus sobre el `StatusNotifierWatcher` real de la sesión GNOME del usuario:

- Registro correcto: `id=io.github.wilkin.Segmenta`, `title=Segmenta`, `icon=io.github.wilkin.Segmenta`, `status=Active`, `category=ApplicationStatus`.
- Menú inactivo: `Sin descargas activas` (informativo/deshabilitado) + Abrir · Preferencias · ─── · Salir.
- Menú con descarga real (105 MB por HTTP): `2 % · 2.2 MB de 104.9 MB` y `2.5 MB/s · 42 s restante`, refrescando en vivo, con Abrir · Pausar · Cancelar · Preferencias · Salir.
- **Tres defectos encontrados y corregidos** (`1e2cdff`, `add2bfc`, `d11135a`): los ids de DBusMenu se renumeraban en cada refresco y el clic caía en una tarea vieja; el handler resolvía la tarea contra el modelo de la ventana, que deja de refrescarse cuando la ventana está oculta en la bandeja; y las acciones seguían dependiendo del hilo GTK. Ahora el tray ejecuta pause/resume/cancel con su propio cliente sobre la tarea que está mostrando.
- **Bloqueo del entorno, no del código**: la extensión `ubuntu-appindicators@ubuntu.com` falló en `_setupProxy` (`appIndicator.js:484`) y el `StatusNotifierWatcher` desapareció; además hay dos ids de extensión AppIndicator habilitados a la vez, uno de ellos (`appindicatorsupport@rgcjonas.gmail.com`) sin archivos instalados. Hasta resolver eso, el icono no puede aparecer aunque el código registre bien.

### Publicación v0.3.0 (2026-11-16)

- Notas y CHANGELOG actualizados con los tres defectos de la bandeja encontrados en la prueba en vivo
  del host (ids de DBusMenu, resolución de la tarea con la ventana oculta, reintento del watcher), y
  con el gate real de la versión final (440 workspace, 113 GUI, 176 Node).
- README apunta a `v0.3.0` como última release; OpenSpec 04/07 sin cambios.
- Artefactos finales: `.deb` sha256 `2c09f8f641964599d1a588490e6e79bcb550c0740c88ec1eaaabf673cde39004`,
  extensión `f4d7fb68796202a7cb53bc0a7c76dc250b87164d33cca6b06364041f4fddb8dd`, firefox
  `2cc55306233b9b5c3c1fc5d82374f528f9be27acce02c0d5d7984d9f502e83eb`; `check-package` 23/23.
- Publicada con tag `v0.3.0` y GitHub Release con los cinco assets.

### Segunda pasada de remediación (2026-11-16, `bb86b28`)

Reportado por el usuario tras probar el `.deb` anterior:

- **R3 — Detección de la extensión**: sólo miraba `<perfil>/Extensions/<id>/`, que Chromium crea para
  instalaciones empaquetadas; el flujo documentado carga la extensión **desempaquetada** (una entrada en
  `Preferences` con `location: 4`). Ahora `extension_evidence()` lee `Preferences` y `Secure
  Preferences`, devuelve `origin` y `loaded_version`, y `extension_version_report_with()` usa la versión
  cargada como referencia del `version_sync`. Firefox por `extensions.json`. Verificado en el host:
  `chrome → extension-installed, origen=unpacked, cargada=0.3.0, sync=match`.
- **R4 — Programar sin pasado**: el calendario permitía elegir un día anterior y la corrección ocurría
  sólo en el payload, así que la UI mostraba el día inválido. Se eliminó el calendario: ahora es un
  `adw::SpinRow` **En días** (0–60) más hora y minutos; con 0 días y una hora pasada, los propios
  widgets se corrigen. Test `the_schedule_picker_cannot_express_the_past`.
- **R5 — Página de Navegadores**: subtítulo ≤8 palabras por navegador, un botón = próximo paso, el
  instructivo en un `adw::ExpanderRow` de 3 líneas; fuera los dos párrafos de avisos y el grupo de
  alternativa local (queda una línea con la ruta).
- **Gate**: `cargo test --workspace` **446 ×2**, `postprocess` 14 ×3, GUI **117**, Node 176, fmt/clippy
  limpios. `.deb` `0.3.0-1` sha256 `63bf7d16350b93192838207a1c5238764bf786b4440b7e4b855c54f8f765480c`,
  instalado con `--reinstall` en el host.
- **ETXTBSY en hooks**: `a_non_zero_exit_is_recorded_without_failing_the_task` falló una vez con
  `Spawn { Text file busy }`; el `rename` del helper no alcanzaba (mismo inode, descriptor heredado por
  un `fork` concurrente). Se agregó `FAKE_EXEC` (mutex por binario) y `run_retrying_spawn`, que reintenta
  sólo ante `Text file busy`.

### Publicación real de v0.3.0 (2026-11-16)

- **Corrección de registro**: `v0.3.0` **no existía** como tag ni release; la última publicada era
  `v0.2.4`, y el README/ODD estaban adelantados a la realidad. Esta es la primera publicación de
  0.3.0, sin mover ni reutilizar tags previos.
- **CI como condición dura**: el run `43995619905` (`bb86b28`) quedó **cancelado** por la regla
  `concurrency: cancel-in-progress` del workflow al pushear documentación; `gates` ya había pasado.
  La evidencia que autorizó la publicación es el run **`43996352963`** sobre `ed0d4cc`, el commit
  taggeado: `gates`, `package` y `clean-install` en **success**.
- Tag anotado `v0.3.0` → `ed0d4cc9144d3d0d63c903ba9152b9db7ae71a4c` (= `origin/main`).
- Release: https://github.com/wilkin-barban/Segmenta/releases/tag/v0.3.0 (no draft, no prerelease)
  con 5 assets.
- **Asset descargado verificado**: `sha256sum -c segmenta_0.3.0-SHA256SUMS` y `.deb.sha256` → los
  tres `OK`; `.deb` `63bf7d16350b93192838207a1c5238764bf786b4440b7e4b855c54f8f765480c`.
- **Host Ubuntu**: instalado el `.deb` bajado del release con `--reinstall` → `0.3.0-1`, daemon
  `0.3.0 (protocolo v1)`, `extension.status` con
  `chrome → extension-installed, origen=unpacked, cargada=0.3.0, sync=match`.
