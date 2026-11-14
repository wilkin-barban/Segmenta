# Segmenta 0.2.1

Correcciones de interfaz y gestión segura de descargas. **Sin publicar:** se etiqueta y se
publica sólo cuando el QA real en Ubuntu cierre y exista el `.deb` 0.2.1.

## Interfaz

- **Preferencias vuelve a mostrar sus siete páginas**: General, Conexiones y límites,
  Descargas web, Navegadores, Antivirus, Site logins y Acerca. La página **Antivirus** existía
  desde 0.2.0 pero nunca se añadía al diálogo; ahora está y un test fija el conjunto completo.
- El diálogo abre a **960×760**. Con 640 px Adw colapsaba a la barra inferior de pestañas, que
  recortaba las últimas páginas y aplastaba los títulos de fila hasta una letra por línea.
- Las filas de Navegadores llevan un ancho mínimo: dos o tres botones de sufijo ya no convierten
  el nombre del navegador en una columna vertical.
- El **spinner de carga** ya no se pinta sobre una lista con datos: volver a `Loading` en una
  reconexión no vuelve a cubrir la lista.

## Gestión segura de descargas

- **Limpiar completadas**: borra únicamente filas `completed` y **no toca ningún archivo**. Con
  cero completadas avisa y no hace nada.
- **Eliminar seleccionadas** (una o varias): modo de selección con casilla por fila, diálogo que
  indica cuántas tareas y archivos se verán afectados y casilla **«Eliminar también los archivos
  descargados»** desmarcada por defecto.
- El diálogo no se cierra hasta conocer el resultado e informa por tarea: filas borradas,
  archivos borrados y cada fallo por nombre.
- Un archivo que no se puede borrar **no se reporta como éxito**: la tarea queda `failed` con
  `1048 ERR_FILE_DELETE_FAILED` y su ruta aparece en el informe.
- Una tarea activa se detiene y se espera al motor antes de tocar su archivo.

## Lista y progreso

- La lista conserva el orden `created_at DESC` del daemon: la descarga más reciente queda arriba,
  no al final.
- La barra de progreso avanza con cada `task_progress`. `yt-dlp` 2026.10.26 emite el template sin
  el prefijo `download:`; el parser ya acepta esa forma real, y los streams HLS/DASH o torrents sin
  tamaño conocido usan modo indeterminado con los bytes descargados visibles.
- **iCloud**: acepta la respuesta `200` con `Content-Range` que iCloud devuelve para un rango
  completo, en vez de reiniciar dos veces y fallar con `1028`.
- Si una descarga pequeña termina antes de que la confirmación pueda pausarla, la extensión avisa
  con una notificación y permite añadir la URL a Segmenta; no la deja en silencio.

## Extensión

- **YouTube tras navegación SPA**: el content script podía reportar `/` o el origen y
  `validateSender` lo rechazaba con `ERR_INVALID_SENDER`, que la interfaz muestra como «La
  solicitud no es válida para la extensión». Ahora se acepta el origen exacto
  `https://www.youtube.com` y el URL del payload se sigue validando por separado.
- **Allowlist de streams**: con `allowStreams` activo y allowlist vacía se aceptaba cualquier
  host; ahora una allowlist vacía es deny-by-default.
- **Instalación real**: el perfil de Chrome tenía cargada una copia **0.1.1** desde
  `~/.local/share/segmenta/extension/chromium`, no la 0.2.x versionada. Esa copia antigua no
  entiende el contrato de mensajes actual y produce el mismo error. La carpeta se actualizó a
  0.2.1 con backup de la 0.1.1; hay que pulsar **Recargar** en `chrome://extensions`.

## Límites conocidos

- La captura de archivos depende del navegador: Chrome estable no instala la extensión sin
  tienda ni política, y Firefox snap no puede ejecutar el host nativo.
- La confirmación visual del fix de sender en Chrome real queda pendiente de la recarga de la
  extensión; el panel de Firefox y el cold-start ya tienen capturas reales de `agy`.
