# Segmenta 0.2.1 — remediación de UI, extensión y gestión segura de descargas

## Contexto

La Release 0.2.0 está publicada, pero el usuario encontró dos regresiones al probarla en Ubuntu:

1. **Preferencias con layout roto**: el diálogo sí renderiza, pero deja un spinner persistente sobre la lista/diálogo, solapa y recorta las etiquetas del tab bar inferior, muestra la página Navegadores con texto vertical de ~1 carácter y columnas vacías, recorta “Guardar login” y los controles +/-, y no incluye la página Antivirus. La captura negra inicial era un artefacto de Wayland/Xvfb; las capturas X11 reales son `/tmp/seg-main-x11.png`, `/tmp/seg-prefs-x11.png`, `/tmp/seg-prefs-conexiones-y-limites.png`, `/tmp/seg-prefs-navegadores.png` y `/tmp/seg-prefs-contraseñas-de-sitios.png`.
2. **Extensión en YouTube**: el usuario ve el texto “La solicitud no es válida para la extensión.” Ese texto corresponde a `errorInvalidMessage` en `_locales/es`. La causa real es `ERR_INVALID_SENDER`: `validateSender` exigía que `sender.url` fuera `/watch` o `/playlist`, pero tras una navegación SPA el content script puede reportar `/` o el origen. Corregido en `7dfda2c`; la confirmación en un navegador real sigue pendiente.

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

**Estado:** causa reproducida sin navegador con `sender.url = https://www.youtube.com/` y payload válido: `ERR_INVALID_SENDER`. `7dfda2c` acepta el origen exacto y conserva la validación del payload; `944782a` cierra el bypass de allowlist. 168 tests Node verdes. Falta la confirmación visual en Chrome real/CFT; el panel de Firefox ya se capturó con `agy`.

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

- Preferencias X11 antes/después: `/tmp/seg-0215-prefs.png` … `/tmp/seg-0216-nav-bottom.png`.
- Feature en la app instalada: `/tmp/seg-0216-clean-confirm.png`, `/tmp/seg-0216-cleaned.png`,
  `/tmp/seg-0216-deleted.png` y `/tmp/seg-0216-deleted-files.png`.
- Cold-start Chromium y panel de Firefox capturados por `agy` en
  `docs/qa/screenshots/cold-start-confirm-run{1,2,3}.png` y `firefox-youtube-panel.png`.
- Informe consolidado: `docs/qa/segmenta-0.2.1-live.md`.
- La Release 0.2.0 está publicada; cualquier corrección va a 0.2.1.

## Estado

| Unidad | Commits | QA real |
| --- | --- | --- |
| P0 Preferencias: página Antivirus ausente | `15aa668` | confirmado por captura |
| P0 Preferencias: ancho, títulos verticales, controles recortados | `82832cf`, `787b8bc` | confirmado por captura |
| P0 spinner central | `82832cf`, `d2dd04e`, `326a754`, `8477503` | confirmado: flecha normal, sin glifo |
| Feature daemon: limpiar completadas y borrado por lote | `763bf14`, `fb1992b` | — |
| Feature GUI: menú, selección múltiple, diálogo, i18n | `976bf70` | confirmado |
| Bloqueo del diálogo tras la respuesta | `b75d39b` | confirmado: sin deadlock |
| Preferencias: inset inferior, búsqueda negra, descripciones cortadas | `71e39bfe` (build) | confirmado por captura |
| Documentación | `13eca06` | — |
| P0 extensión `ERR_INVALID_SENDER` | `7dfda2c` | reproducido en Node y confirmado en CFT 157; falta recargar el Chrome real |
| Bypass de allowlist de streams | `944782a` | tests negativos de probe/add con host ajeno |

Artefacto final: `segmenta_0.2.1-1_amd64.deb`, sha256
`57fa7a195c1ed9320a742f0412f5fff1bdb8d1f7bcb7fbb52f9457cd8fe00ffb`; `check-package` 22/22.
Sin tag y sin Release.

## Log

- L1: la captura negra de Preferencias era Wayland/Xvfb, no un diálogo vacío. Con capturas X11
  reales los defectos fueron otros: faltaba la página Antivirus (existía sin añadirse), el
  diálogo a 640 px colapsaba a la barra inferior de pestañas y aplastaba los títulos de fila, y
  el spinner central seguía pintándose.
- L2: el spinner no era estado de `refill` ni una segunda ventana: seguía al puntero. El binario
  no contenía ningún `GtkSpinner` tras `326a754`, y el glifo era el cursor heredado de la sesión.
  `8477503` fija el cursor por defecto y `SEGMENTA_DEBUG_TREE=1` quedó como diagnóstico para
  nombrar cualquier widget que se pinte en el centro.
- L3: el bloqueo tras "Limpiar completadas" no era deadlock del worker: `can_close(false)` también
  bloquea el `close()` programático, así que el diálogo sobrevivía con el estado ya tomado y sus
  handlers hacían `return`. `b75d39b` resetea el pendiente y restaura `can_close` antes de cerrar.
- L4 (QA real del usuario sobre `71e39bfe`): limpiar completadas con 1 tarea cerró el diálogo y
  conservó el archivo; multi-delete de 2 tareas conservó archivos; delete de 1 tarea con la
  casilla marcada borró el `.zip` e informó "se borraron 1 archivo(s)"; sin deadlock. Preferencias:
  búsqueda negra eliminada, descripciones con wrap y scroll hasta el final por encima de la tab bar.

- L5: `ERR_INVALID_MESSAGE` era `ERR_INVALID_SENDER`. Reproducido sin navegador: con
  `sender.url = "https://www.youtube.com/"` —lo que reporta el content script tras una navegación
  SPA— `validateInbound` devolvía `ERR_INVALID_SENDER`, y el content script lo traduce al mismo
  texto genérico. `validateSender` acepta ahora el **origen exacto** `https://www.youtube.com`
  (comprobado con `new URL`: scheme `https` y host exacto) y sigue validando el `url` del payload
  por su cuenta. Tests: `/`, origen sin barra, `/results` y `/watch` aceptados; `evil.example`,
  `http://`, `m.youtube.com` y `www.youtube.com.evil.example` rechazados.
- L6 (corregido en la misma unidad): `isAllowedUrl` aceptaba **cualquier host** cuando
  `allowStreams` estaba activo y la allowlist vacía (`protocol.js:180-181`), y el background pasa
  `allowStreams: true`, así que `https://evil.example/watch?v=1` pasaba `segmenta.probe kind=stream`
  y `segmenta.add`. Ahora una allowlist vacía es **deny-by-default**: sólo YouTube exacto, o el host
  explícitamente listado. Tests negativos para probe stream y add con host ajeno, y los positivos
  pasan con el host en la allowlist.

- L7 (causa de la instalación real): el perfil de Chrome cargaba la extensión **0.1.1** desde
  `~/.local/share/segmenta/extension/chromium`, no la versión 0.2.x. Esa copia antigua no entiende
  el contrato actual, y produce el mismo texto de error. La carpeta se reemplazó por 0.2.1 con
  backup en `chromium-0.1.1-backup-20261113`; falta pulsar **Recargar** en `chrome://extensions`
  y repetir el panel.

## Pendiente

1. **Recargar la extensión en el Chrome real y verificar el panel de YouTube.** La carpeta ya es
   0.2.1; la prueba CFT con la misma carpeta aceptó el sender y respondió `ERR_HOST_UNAVAILABLE`,
   no `ERR_INVALID_SENDER` (`docs/qa/segmenta-0.2.1-cdp-sender.txt`). Falta que el Chrome del
   usuario recargue la copia y se confirme visualmente.
2. El informe y las capturas de `agy` están en su commit `1ac4799`; las capturas también se
   guardaron en `docs/qa/screenshots/`.


