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

- [ ] T1 — Velocidad real en torrent y streams HLS/DASH (S1, S4) — `crates/segmenta-daemon/src/{manager,torrent,stream_media}.rs` — commit `fix(daemon): report real speed for torrents and streams`
- [ ] T2 — Velocidad dedicada en la fila de la GUI (S1, S4) — `crates/segmenta-gui/src/{model,window}.rs`, `crates/segmenta-i18n/src/lib.rs`, tests GUI — commit `feat(gui): show the download speed in the list row`
- [ ] T3 — `scheduled_at` por tarea: migración, RPC y arranque diferido (S3, S5) — `crates/segmenta-daemon/src/{store,manager,scheduler}.rs` — commit `feat(daemon): schedule one download for an exact time`
- [ ] T4 — Diálogo **Programar** y estado visible en la fila (S3, S5, S7) — `crates/segmenta-gui/src/{add_dialog,window,model}.rs`, i18n, tests — commit `feat(gui): schedule a download from the interface`
- [ ] T5 — Bandeja del sistema con progreso y acciones (S2, S6, S7) — `crates/segmenta-gui/src/tray.rs` (nuevo), `Cargo.toml`, `main.rs`, `app.rs`, `window.rs`, i18n — commit `feat(gui): add a system tray icon with live progress`
- [ ] T6 — Documentación (OpenSpec 04/07, README, CHANGELOG, notas 0.3.0) — commit `docs: document 0.3.0`
- [ ] T7 — Artefactos 0.3.0 y verificación en el host (sin publicar) — commit `chore(release): bump to 0.3.0`

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
