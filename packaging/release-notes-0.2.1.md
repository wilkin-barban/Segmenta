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

## Límites conocidos

- La captura de archivos depende del navegador: Chrome estable no instala la extensión sin
  tienda ni política, y Firefox snap no puede ejecutar el host nativo.
- El error de mensajería de la extensión visto en un perfil real todavía está en investigación;
  no se publica 0.2.1 con ese punto abierto.
