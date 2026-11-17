<p align="center">
  <img src="https://raw.githubusercontent.com/wilkin-barban/Segmenta/main/packaging/io.github.wilkin.Segmenta.svg" width="96" alt="Icono de Segmenta">
</p>

# Segmenta 0.3.0

Velocidad real visible en la fila de descargas y en descargas de medios/torrents, programador de
descargas por fecha y hora exactas, e icono en la bandeja de sistema adaptado a Ubuntu con ciclo de
vida en segundo plano.


## Qué problema resuelve

Esta release aborda tres limitaciones clave de la experiencia diaria de uso:

1. **Velocidad no visible o falseada en cero.** En la interfaz principal de Segmenta la velocidad de
   descarga quedaba relegada a un texto secundario que solía truncarse; además, las descargas de
   torrents y streams HLS/DASH publicaban internamente `speed_bps: 0`, dejando al usuario sin
   información sobre la tasa real de transferencia.
2. **Imposibilidad de programar una descarga puntual.** Aunque existía la configuración de una
   ventana diaria por cola (`queue.schedule`), no era posible indicar que un archivo específico se
   descargara a una hora o fecha puntual (por ejemplo, esta noche a las 23:30 o mañana por la
   madrugada) sin alterar el horario global de la cola.
3. **Falta de presencia en la bandeja del sistema y persistencia al cerrar.** Al cerrar la ventana
   de Segmenta se terminaba la interfaz gráfica, obligando a mantener la ventana abierta para seguir
   el estado o interactuar con las descargas en ejecución. Faltaba una integración nativa con la
   bandeja de Ubuntu que permitiera ver el progreso y operar tareas sin ocupar espacio en la barra
   de tareas.

## Cambios

### Velocidad real y dedicada en la fila

`ProgressPipeline` en `crates/segmenta-daemon/src/manager.rs` unifica el cálculo de tasa de
transferencia a partir del incremento de bytes entre muestras sucesivas y el intervalo de tiempo
transcurrido, computando a la vez el tiempo estimado restante (ETA). Las descargas de torrents y
streams HLS/DASH dejan de publicar `speed_bps: 0` y reportan su velocidad real.

En la GUI (`crates/segmenta-gui/src/window.rs`), cada fila de descarga incorpora una etiqueta de
velocidad dedicada a la derecha, atenuada y con nombre accesible derivado de `Msg::Speed`. El método
`sync_row` actualiza el valor en el lugar sin reconstruir la jerarquía de widgets de la fila, y la
etiqueta se oculta de forma automática cuando la velocidad es cero.

### Descargas programadas por fecha y hora (`scheduled_at`)

La migración aditiva 6 en SQLite añade la columna `scheduled_at TEXT` (protegida por verificación
`has_column`, sin requerir reescritura de tabla). Los métodos RPC `download.add` y `media.add`
admiten el parámetro `scheduled_at`, aceptando:
- Formato ISO-8601 con zona horaria explícita (ej. `2026-11-16T02:00:00Z` o con offset).
- Formato `HH:MM` local (hoy, o mañana si la hora indicada ya transcurrió en el huso horario local,
  resolviendo transiciones horarias y DST con `mktime`).

Cualquier otro formato se rechaza con `1004 ERR_INVALID_PARAMS` indicando el formato esperado. El
valor se normaliza internamente a UTC RFC-3339.

Una tarea cuya fecha/hora programada sea futura permanece en estado `pending`. El motor respeta esta
restricción: `start_pending_for_queue` la omite, `download.resume` responde manteniéndola en
`pending`, y el relanzamiento multimedia preserva su programación. El tick de 30 segundos del
planificador (`scheduler.rs`) inicia la descarga automáticamente al cumplirse el momento previsto.
La programación diaria por cola (`queue.schedule`) se mantiene intacta y opera de forma
complementaria.

### Icono en la bandeja del sistema (System tray)

`crates/segmenta-gui/src/tray.rs` implementa la especificación StatusNotifierItem mediante la
biblioteca pura en Rust `ksni` 0.3. No añade dependencias en C ni paquetes adicionales a los
requerimientos del sistema o del empaquetado `.deb`.

En Ubuntu (donde la extensión *Ubuntu AppIndicators* viene habilitada por defecto en GNOME), el clic
izquierdo despliega el menú de la bandeja, el cual funciona como el widget informativo:
- **Cabecera**: nombre del archivo de la descarga activa (o identificador si no tiene nombre, o
  indicador de inactividad si no hay tareas activas).
- **Progreso**: porcentaje y bytes descargados sobre el total (un total desconocido se renderiza
  como `—` en lugar de 0%).
- **Velocidad y ETA**: tasa de transferencia y tiempo restante formateados.
- **Acciones**: **Abrir Segmenta**, **Pausar** o **Reanudar** (según el estado de la tarea activa),
  **Cancelar**, **Preferencias** y **Salir de la aplicación**.
- Soporte completo de localización en español, inglés y portugués (`es`, `en`, `pt_BR`).

La bandeja corre en su propio hilo con un cliente IPC independiente, escucha eventos de tareas
(`task_progress`, `task_added`, `task_updated`) con refresco limitado a 1 segundo y cuenta con un
sondeo de respaldo cada 2 segundos.

### Ciclo de vida: cerrar a la bandeja

Cuando la bandeja de sistema se inicializa correctamente, cerrar la ventana principal de Segmenta ya
no finaliza el proceso: la ventana se oculta y la aplicación permanece viva mediante `app.hold()`,
permitiendo que las descargas continúen y el icono permanezca disponible. La opción **Salir de la
aplicación** desde el menú de la bandeja es la vía para terminar el proceso de forma definitiva.

Si el entorno no dispone de un servicio de bandeja (sin `StatusNotifierWatcher` activo en D-Bus),
cerrar la ventana finaliza la aplicación de forma inmediata, manteniendo el comportamiento previo sin
dejar procesos huérfanos. Los comandos enviados desde el hilo de la bandeja cruzan al hilo principal
de GTK mediante un canal seguro que se vacía cada 250 ms.

### Bandeja probada en vivo: tres defectos corregidos

Probar el icono en un escritorio GNOME real destapó problemas que ninguna prueba sintética mostraba:

- **El menú actuaba sobre la tarea equivocada.** Cada refresco reconstruía el menú y cada ítem
  capturaba el `task_id` del momento en que se construyó, así que un clic podía pausar una descarga
  anterior o no hacer nada. Las acciones ya no llevan `task_id`: resuelven la tarea activa en el
  instante del clic, y los identificadores de DBusMenu quedaron estables entre refrescos (verificado
  por D-Bus: ids `7…15` idénticos a dos segundos de distancia).
- **La ventana oculta rompía Retomar y Cancelar.** El handler resolvía la tarea contra el modelo de
  la ventana, que deja de refrescarse cuando está oculta en la bandeja. Ahora el tray ejecuta pausa,
  reanudación y cancelación con su propio cliente IPC, sobre la tarea que el menú está mostrando.
- **El icono no aparecía si el watcher llegaba tarde.** `StatusNotifierWatcher` pertenece al shell,
  no a Segmenta: falta en una sesión sin la extensión AppIndicator, mientras esa extensión carga y
  durante un instante tras reiniciar el shell. Antes se rendía en el primer intento y la aplicación
  quedaba sin icono para siempre; ahora reintenta cada 5 segundos hasta lograrlo.

Ciclo verificado en el host con una descarga HTTP real de 105 MB, operando el menú por D-Bus:
pausar (`downloading` → `paused`), retomar (`paused` → `downloading`) y cancelar
(`downloading` → `cancelled`), y el menú volviendo a `Ningún download activo` al quedar sin tareas.

### Dos pantallas que estorbaban, corregidas

- **Preferencias → Navegadores** apilaba tres botones de texto como sufijos de cada fila, y en el
  ancho real del diálogo el subtítulo terminaba envuelto una letra por línea. La fila ahora muestra
  un único botón primario —*Preparar*, o *Recargar* cuando la copia preparada no es la enviada— y
  mueve el resto a un menú de tres puntos. La versión queda como etiqueta atenuada.
- **El diálogo de descarga** tenía dos interruptores que se contradecían: *Comenzar ahora* y
  *Programar*. Se eliminó el primero, porque sin programación la descarga ya empieza de inmediato.
  *Programar* despliega un único selector *Cuándo* con opciones rápidas —30 minutos, 1 hora,
  2 horas, esta noche, mañana a las 08:00— y solo *Fecha y hora exacta…* abre el calendario y la
  hora. Una fila de resumen dice cuándo se va a descargar, y una fecha u hora pasada se corrige al
  instante válido más cercano en lugar de enviarse al daemon.

### Estabilidad en el suite de pruebas

Se corrigieron dos condiciones de carrera preexistentes en los tests:
- `keyring_is_detected_without_the_version_flag`: absorbía un race transitorio `exec`/`ETXTBSY` sobre
  el binario de prueba de `secret-tool`; ahora reintenta la sonda.
- `system_open_launches_the_gui_with_the_task_as_an_argument`: serializa el acceso a la variable de
  entorno global `SEGMENTA_GUI` mediante un mutex para garantizar aislamiento entre pruebas concurrentes.

## Cómo se usa

### CLI: programar descargas con `--at`

```bash
# Programar una descarga para una hora local específica hoy (o mañana si ya pasó)
segmenta add https://example.com/ubuntu-26.04.iso --at 23:30

# Programar con fecha y hora exacta en formato ISO-8601 con zona
segmenta add https://example.com/archivo.tar.gz --at 2026-11-16T03:00:00Z

# Programar una descarga multimedia
segmenta media add https://www.youtube.com/watch?v=dQw4w9WgXcQ --mode video --height 1080 --at 01:15
```

### GUI: diálogo de alta y fila de descargas

1. Al abrir el diálogo de añadir descarga (`Ctrl+N` o botón **+ Añadir URL**), activar el
   interruptor **Programar**.
2. Seleccionar la fecha deseada en el calendario integrado y ajustar los selectores de hora y
   minuto.
3. Pulsar **Iniciar** (o `Enter`): la tarea se creará en estado pendiente y la fila mostrará el
   texto «Programada para [fecha/hora]» junto con el botón de cancelar. Al llegar la hora señalada,
   la descarga comenzará automáticamente.

### Bandeja del sistema

- El icono de Segmenta aparece en el panel superior de Ubuntu al iniciar la aplicación gráfica.
- Al hacer clic sobre el icono, se despliega el menú con la información en vivo de la descarga
  principal: archivo, porcentaje, velocidad y tiempo estimado.
- Las acciones **Pausar** y **Reanudar** se alternan dinámicamente según el estado de la tarea.
- Cerrar la ventana principal con el botón de cerrar (`X`) oculta la ventana sin detener la
  transferencia. Para restaurar la ventana, seleccionar **Abrir Segmenta** en el menú.
- Para cerrar completamente la aplicación, seleccionar **Salir de la aplicación** en el menú de la
  bandeja.

## Verificación

Resultados observados en el árbol de desarrollo:

- `cargo test --workspace` → 443 tests verdes (repetido dos veces sobre el árbol final).
  `ETXTBSY` del arnés).
- `xvfb-run -a cargo test -p segmenta-gui` → 116 tests verdes.
- `node extension/tests/run.mjs` → 176 tests verdes.
- `cargo fmt --all -- --check` y `cargo clippy --workspace --all-targets -- -D warnings` limpios.
- `segmenta-gui --smoke` OK (arranque, renderizado sintético y salida limpia bajo Xvfb).
- `packaging/check-package.sh dist/segmenta_0.3.0-1_amd64.deb 0.3.0` → `paths=23/23`.

## Instalación desde el código

Desde la release publicada:

```bash
sha256sum -c segmenta_0.3.0-SHA256SUMS
sudo apt install ./segmenta_0.3.0-1_amd64.deb
```

O desde el código:

```bash
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.3.0-1_amd64.deb 0.3.0
sudo apt install ./dist/segmenta_0.3.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

---

### Todas las releases

0.3.0 (en preparación) · [0.2.4](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.4) · [0.2.3](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.3) · [0.2.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.1) · [0.2.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.0) · [0.1.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.2) · [0.1.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.1) · [0.1.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.0) · [0.0.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.2) · [0.0.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1)
