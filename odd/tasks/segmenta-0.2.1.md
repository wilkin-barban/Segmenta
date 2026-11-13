# Segmenta 0.2.1 — remediación de UI, extensión y gestión segura de descargas

## Contexto

La Release 0.2.0 está publicada, pero el usuario encontró dos regresiones al probarla en Ubuntu:

1. **Preferencias con layout roto**: el diálogo sí renderiza, pero deja un spinner persistente sobre la lista/diálogo, solapa y recorta las etiquetas del tab bar inferior, muestra la página Navegadores con texto vertical de ~1 carácter y columnas vacías, recorta “Guardar login” y los controles +/-, y no incluye la página Antivirus. La captura negra inicial era un artefacto de Wayland/Xvfb; las capturas X11 reales son `/tmp/seg-main-x11.png`, `/tmp/seg-prefs-x11.png`, `/tmp/seg-prefs-conexiones-y-limites.png`, `/tmp/seg-prefs-navegadores.png` y `/tmp/seg-prefs-contraseñas-de-sitios.png`.
2. **Extensión en YouTube**: el usuario ve el texto “La solicitud no es válida para la extensión.” Ese texto corresponde a `errorInvalidMessage` en `_locales/es`, usado para `ERR_INVALID_MESSAGE`, `ERR_INVALID_TYPE`, `ERR_INVALID_PARAMS` o `ERR_MESSAGE_TOO_LARGE`. No se reprodujo con Chrome for Testing 157 + `chromium-0.2.0`: el panel abrió y mostró calidades. Hay que reproducirlo con el perfil/instalación reales y corregir la causa, no ocultar el error.

Además, se pide una función nueva de gestión segura de descargas.

## P0 — Layout de Preferencias

- Reproducir con la GUI instalada y con `segmenta-gui` del árbol; usar Xvfb y captura. Revisar `prefs.rs` (`present`, `load_site_logins`, `site_logins_page`, `antivirus_page`) y el orden de construcción.
- Determinar si el diálogo queda bloqueado por una llamada sincrónica, un hilo que no vuelve, una página mal añadida o un fallo GTK.
- Corregir manteniendo las páginas reales: General, Red, Captura, Navegadores, Antivirus, Site logins y Acerca.
- La apertura no debe bloquear el hilo GTK; si falla una consulta, mostrar estado de error y permitir reintentar.
- Tests GUI: abrir Preferencias, verificar páginas/controles, cambiar una opción, site login vacío y no vacío, antivirus visible. Prohibido test que sólo compruebe que el diálogo existe.

## P0 — Error de extensión “La solicitud no es válida para la extensión.”

- Reproducir con el perfil real y con CFT. Capturar la consola del service worker y el `code` exacto (`ERR_INVALID_MESSAGE`, `ERR_INVALID_TYPE`, `ERR_INVALID_PARAMS`, `ERR_MESSAGE_TOO_LARGE` u otro).
- Verificar ID de extensión (`kdeldnifdbpemnhfpmhonlnibbhnoaan`), manifiesto nativo, `allowed_origins`, ruta de `segmenta-host`, `SETTINGS_CACHE_VERSION` y `validateInbound`.
- Revisar `validateSender`/`validateProbe` y la construcción del mensaje del panel; no relajar la validación para “hacerlo pasar”.
- Agregar diagnóstico seguro (sin URL con credenciales ni secretos) si hace falta.
- Tests Node para el caso exacto; prueba real en Chrome normal/CFT y Firefox oficial.

## Feature — Gestión segura de descargas

### A. Limpiar terminadas

- Acción visible en la GUI (barra principal o menú): **“Limpiar completadas”**.
- Debe operar únicamente sobre tareas `completed`.
- **Nunca elimina archivos descargados** (`delete_file=false`).
- Mostrar confirmación con el número de tareas afectadas; si no hay completadas, informar y no hacer nada.
- Refrescar la lista y mostrar cuántas se limpiaron; si una falla, continuar con las demás y reportar el error exacto.

### B. Eliminar seleccionadas, una o varias

- Permitir selección múltiple en la lista (modo selección/checkbox o selección extendida clara).
- Acción **“Eliminar”** con diálogo de confirmación:
  - casilla **“Eliminar también los archivos descargados”** (por defecto desmarcada);
  - texto que indique cuántas tareas y cuántos archivos se verán afectados;
  - para tareas activas: primero pausar/cancelar de forma segura; no borrar un archivo mientras el motor lo escribe.
- Con la casilla desmarcada: elimina sólo el registro/historial; conserva archivos.
- Con la casilla marcada: elimina registros y archivos vinculados; si un archivo no se puede borrar, la tarea debe quedar en estado/error visible y no fingir éxito.
- Funcionar con una sola tarea y con varias; no cerrar el diálogo hasta conocer el resultado.
- Para la API, evaluar una operación batch (`download.delete` con lista o `download.delete_many`) con resultado por tarea; no dejar una eliminación a medias silenciosa. Mantener compatibilidad con la operación individual existente.
- i18n es/en/pt, accesibilidad, atajos y tests GUI/daemon. Confirmación destructiva obligatoria.

## Gates y cierre

- `fmt`, `clippy -D warnings`, Rust workspace, Node extension, `sync --check`, `pack --check`, GUI xvfb.
- QA real en Ubuntu de ambos P0 y de la feature, con capturas y logs.
- Documentar en OpenSpec 04/07/08 y release notes 0.2.1.
- No publicar 0.2.1 hasta que los dos P0 y la feature estén verdes.

## Evidencia actual

- Screenshot de Preferencias en Xvfb: `/tmp/seg-prefs.png` (negro con spinner).
- CFT 157 con `chromium-0.2.0`: panel de YouTube abrió correctamente; no se reprodujo el error de extensión.
- La Release 0.2.0 está publicada; cualquier corrección va a 0.2.1.
