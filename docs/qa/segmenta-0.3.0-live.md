# Segmenta 0.3.0 — QA y verificación en entorno de desarrollo

Fecha: 2026-11-15. Versión: 0.3.0 (en preparación en este árbol de trabajo, sin tag ni Release publicada).
Paquete inspeccionado: `dist/segmenta_0.3.0-1_amd64.deb`.
`check-package` 23/23 sobre `dist/segmenta_0.3.0-1_amd64.deb`.

No se utilizaron capturas de pantalla ni herramientas de automatización de navegador en esta verificación.

## Qué se verificó en este espacio de trabajo

Se verificaron las implementaciones correspondientes a las especificaciones S1 a S7 de la versión
0.3.0 mediante las suites de pruebas unitarias, de integración, de interfaz gráfica bajo pantalla
virtual y validación de empaquetado:

1. **Velocidad de descarga real y visible (S1, S4):**
   - Integración de `ProgressPipeline` en `crates/segmenta-daemon/src/manager.rs` para derivar la
     tasa de transferencia a partir de los bytes recibidos entre muestras y el tiempo transcurrido,
     eliminando los ceros estáticos (`speed_bps: 0`) que publicaban descargas de torrents y streams
     multimedia HLS/DASH.
   - Presencia de la etiqueta dedicada de velocidad en la fila de descargas de la GUI
     (`crates/segmenta-gui/src/window.rs`), verificando actualización en el lugar por `sync_row`
     y ocultamiento automático cuando la velocidad es nula.
2. **Descargas programadas por fecha y hora (S3, S5):**
   - Migración 6 aditiva en SQLite (`scheduled_at TEXT` verificado con `has_column` sin reescritura
     de esquema).
   - Normalización de formatos en `download.add` y `media.add`: aceptación de ISO-8601 con zona
     horaria y formato `HH:MM` local (hoy, o mañana si la hora ya transcurrió en el huso horario
     local con DST vía `mktime`), normalizados a UTC RFC-3339; rechazo estricto con
     `1004 ERR_INVALID_PARAMS` para cualquier otro formato.
   - Retención en `pending` de tareas programadas con fecha futura (`start_pending_for_queue`,
     `download.resume` y relanzamiento multimedia preservan el estado) y arranque automatizado
     mediante el tick de 30 segundos de `scheduler.rs`.
   - Soporte en CLI mediante el argumento `--at` e integración en el diálogo de alta de la GUI.
3. **Bandeja de sistema y ciclo de vida en segundo plano (S2, S6, S7):**
   - Integración de `ksni` 0.3 (StatusNotifierItem puro en Rust sin dependencias C ni librerías de
     sistema adicionales).
   - Modelo puro de menú (`TrayModel`) probado exhaustivamente sin requerir servidor gráfico ni
     D-Bus: jerarquía de filas informativas (nombre, porcentaje, velocidad/ETA) y acciones
     disponibles (abrir, pausar/reanudar, cancelar, preferencias, salir).
   - Manejo seguro de tamaños desconocidos (renderizado como `—` en lugar de 0%).
   - Cliente IPC independiente en hilo secundario con refresco limitado a 1 s ante eventos
     (`task_progress`, `task_added`, `task_updated`) y sondeo de respaldo cada 2 s.
   - Ciclo de vida: al estar la bandeja activa, cerrar la ventana oculta la interfaz y mantiene vivo
     el proceso (`app.hold()`), mientras que la opción «Salir de la aplicación» desde la bandeja
     termina el proceso.
   - Comportamiento de respaldo: si no hay un servicio `StatusNotifierWatcher` disponible en D-Bus,
     cerrar la ventana finaliza la aplicación de inmediato (`close_should_quit`), preservando el
     comportamiento sin bandeja.
   - Despacho de comandos de la bandeja al hilo principal de GTK a través de un canal evaluado
     cada 250 ms.
   - Cadenas localizadas en `es`, `en` y `pt_BR` en `crates/segmenta-i18n`.
4. **Estabilidad de pruebas preexistentes:**
   - Corrección de la condición de carrera `exec`/`ETXTBSY` en
     `keyring_is_detected_without_the_version_flag` mediante reintentos de sonda.
   - Serialización de la variable de entorno global `SEGMENTA_GUI` mediante un mutex en
     `system_open_launches_the_gui_with_the_task_as_an_argument`.

## Gates verificados en el workspace

- `cargo test --workspace` → 436 tests verdes (ejecutado y verificado en cuatro corridas consecutivas).
- `xvfb-run -a cargo test -p segmenta-gui` → 109 tests verdes.
- `node extension/tests/run.mjs` → 176 tests verdes.
- `cargo fmt --all -- --check` → limpio (código formateado según las reglas del proyecto).
- `cargo clippy --workspace --all-targets -- -D warnings` → limpio (cero advertencias).
- `segmenta-gui --smoke` → OK (arranque, construcción de ventana con descarga sintética y salida limpia bajo Xvfb).
- `packaging/check-package.sh dist/segmenta_0.3.0-1_amd64.deb 0.3.0` → `paths=23/23` (todas las rutas obligatorias presentes en el paquete Debian).

## Elementos que requieren sesión de escritorio del usuario para inspección en vivo

Los siguientes componentes fueron validados mediante pruebas automatizadas y contratos de datos,
pero requieren una sesión gráfica interactiva real en el escritorio del usuario (GNOME sobre Wayland
o X11) para su observación visual en vivo:

1. **Icono y menú de la bandeja del sistema (System Tray):**
   - En entornos headless, contenedores o bajo Xvfb sin un servicio `StatusNotifierWatcher`
     registrado en el bus de sesión D-Bus, el servicio de bandeja detecta la ausencia del watcher y
     se desactiva de forma segura por diseño.
   - La visualización del icono en el panel superior de Ubuntu y el despliegue del menú dinámico con
     el clic izquierdo dependen de la extensión *Ubuntu AppIndicators* ejecutándose en la sesión
     gráfica activa del usuario.
2. **Diálogo interactivo de programación con calendario:**
   - La interacción manual con el widget `GtkCalendar` y los selectores de hora/minuto del diálogo
     de alta (`AdwDialog`) requiere una pantalla gráfica interactiva donde el usuario opere el
     puntero y el teclado.
