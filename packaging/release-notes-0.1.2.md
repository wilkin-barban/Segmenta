# Segmenta 0.1.2

Correcciones de estado real de la extensión, orden y foco de las tareas, panel de calidades
acotado y captura de archivos con confirmación. El alcance no cambia: descargas directas
HTTP/HTTPS, YouTube público y HLS/DASH públicos sin cifrar.

## Estado real de la extensión

- La GUI ya no dice «Listo» sólo porque exista un manifiesto nativo: distingue **manifiesto
  nativo listo**, **extensión no instalada**, **instalada** y **conectada**, por navegador.
- El identificador del add-on de Firefox es el que se publica, `segmenta@wilkin.github.io`, y
  un test lo ata al manifiesto real para que no vuelvan a divergir.
- Los manifiestos por usuario respetan `XDG_CONFIG_HOME`, se escriben `0644`, validan el host y
  no sobrescriben un archivo existente en silencio. Después de `install-native` el estado se
  refresca y se informa el resultado de cada navegador.
- `segmenta extension status|prepare|install-native|open`: prepara el ZIP en
  `$XDG_DATA_HOME/segmenta/extensions/<familia>-<versión>`, valida `manifest.json`, `_locales` e
  identificador, y abre `chrome://extensions`, `about:debugging` o la carpeta con argv fijo, sin
  shell. Nunca se escriben perfiles ni se usa `--load-extension` en Chrome estable.
- La página Navegadores ofrece estado real, «Preparar extensión», «Abrir página de
  extensiones», «Copiar ruta» y los pasos de carga manual.

## Orden y foco

- Una tarea nueva aparece al principio de la lista, y recargar conserva el orden `created_at DESC`
  del daemon.
- `system.open` trae Segmenta al frente y enfoca la tarea (`segmenta-gui --show-task <id>`), con
  argv fijo y sin shell. La extensión lo invoca después de añadir y muestra «Añadida a Segmenta»
  con la carpeta de destino.

## Panel de calidades

- `media.probe` deduplica los formatos por altura y expone una escalera acotada: 4K (2160), 1080p,
  720p, 360p y solo audio, sólo las disponibles. El valor por defecto es **720p**, no 4K.
- El vídeo se selecciona por `height`, nunca por un `format_id` crudo, también en HLS/DASH.
- Tras «Descargar», la tarjeta pasa a «Enviado a Segmenta» con [Abrir Segmenta] y [Ver progreso],
  en lugar de dejar la lista de formatos abierta.

## Captura de archivos

- Una descarga de `.zip`, `.rar`, `.deb` y compañía ya no se cancela antes de confirmar: se
  **pausa** y se abre una ventana propia con el nombre, el host y la carpeta de destino.
- Aceptar crea la tarea en Segmenta y sólo entonces cancela y borra la descarga del navegador.
  Rechazar, el timeout (~120 s), cerrar la ventana, un fallo del host o una descarga que no se
  puede pausar **reanudan** la descarga del navegador: no se pierde ningún archivo.
- Un watchdog con `alarms` (permiso nuevo) vigila cada captura pendiente: se guarda antes de
  pausar y se rehidrata al despertar el service worker o al arrancar el navegador, de modo que
  ninguna descarga quede pausada si el worker se suspende, vence el plazo o falla el host. Si el
  registro o el alarm no se pueden crear, no se pausa nada.
- `capture_confirm` (por defecto `true`) permite volver al flujo sin ventana manteniendo el orden
  seguro. La lista por defecto de extensiones capturadas se amplía con `deb`, `rpm`, `appimage`,
  `xz`, `whl`, `jar`, `epub`, `vsix`, `xpi` y `msix`.
- `download.add` y `torrent.add` devuelven `destination_dir` y `final_path`, y el evento
  `task_added` los incluye, para que la ventana muestre la carpeta efectiva.

## Compatibilidad

- Una configuración escrita por 0.1.0 o 0.1.1 se carga igual: conserva los valores del usuario y
  gana los defaults nuevos (`capture_confirm`, lista ampliada).
- Sin cambios de esquema que rompan tareas existentes.
