# Segmenta 0.1.2 — estado real de la extensión, foco y captura con confirmación

Sin tag ni Release hasta el QA real del usuario en Ubuntu. Rama `main`, commits pequeños, gates por fase.

## Specs

- **S1.** La UI nunca dice "Listo" sin calificar: distingue **manifiesto nativo listo**, **extensión no instalada**, **instalada** y **conectada**; el estado se comprueba, no se asume por la existencia del manifiesto.
- **S2.** El add-on de Firefox es `segmenta@wilkin.github.io`; el identificador del daemon y el manifiesto no pueden divergir (test que los ata).
- **S3.** Los manifiestos por usuario respetan `XDG_CONFIG_HOME`, se escriben `0644`, validan el host y no sobrescriben un archivo existente sin decirlo; tras `install-native` el estado se refresca y se informa por navegador.
- **S4.** `extension.prepare` descomprime el ZIP en `$XDG_DATA_HOME/segmenta/extensions/<family>-<version>` y valida `manifest.json`, `_locales` e ID; `extension.open` abre `chrome://extensions`, `about:debugging` o la carpeta con argv fijo y sin shell; `extension.status` informa manifiesto de sistema/usuario, detecta la extensión en perfiles Chrome/Firefox en modo lectura y marca snap/Flatpak como no soportado. Nunca se escriben perfiles ni se usa `--load-extension` en Chrome estable.
- **S5.** CLI `segmenta extension status|prepare|install-native|open` y `DemoClient` para la GUI.
- **S6.** La página Navegadores ofrece estado real, "Preparar extensión", "Abrir página de extensiones", "Copiar ruta" y pasos claros; no afirma que está instalada si no lo está.
- **S7.** `DownloadStore::upsert` inserta al inicio; `reload` conserva `created_at DESC` del daemon.
- **S8.** Toda tarea nueva (extensión/CLI/GUI) trae Segmenta al frente y enfoca la tarea: `focus_task` (reset de filtro/búsqueda, selección, detalle, `present`) o `system.open`/`segmenta-gui --show-task <id>`; la extensión lo invoca tras `media.add`/`download.add`, con toast "Añadida a Segmenta" y su carpeta.
- **S9.** El panel de calidades agrupa y deduplica formatos; opciones fijas 4K (2160)/1080p/720p/360p/Solo audio, sólo disponibles y como máximo esas, default **720p**; el mejor muxed/DASH por altura y audio por bitrate. El botón Descargar crea la tarea y pasa la tarjeta a "Enviado a Segmenta" con [Abrir Segmenta]/[Ver progreso]; la lista de formatos no queda interminable. `media.add` video usa `height`, no `format_id`; `add_stream_task` respeta `height`.
- **S10.** Captura de archivos: **no** se cancela antes de confirmar. Pausar (`downloads.pause`), abrir `confirm.html` (`windows.create`) con nombre, host y carpeta destino; aceptar → cancelar + erase + `download.add`; rechazar/timeout → `resume`; si el host falla se reanuda la descarga del navegador (nunca se pierden datos). La ventana muestra la carpeta efectiva.
- **S11.** `download.add`/`torrent.add` devuelven `destination_dir`/`final_path` e informan en `task_added`; setting `capture_confirm` (default true) y extensiones ampliadas (deb, rpm, appimage, xz, whl, jar, epub, vsix, xpi, msix), con tests/manifiestos/cache version actualizados. Después de capturar se abre/enfoca Segmenta y se muestra la tarea.

## Tasks

| ID | Spec | Entrega | Ruta | Commits | Estado |
| --- | --- | --- | --- | --- | --- |
| T1 | S2 | ID de Firefox corregido y atado al manifiesto | inline | `1dbb310` | hecho |
| T2 | S1, S3, S6 | Estado real por navegador, XDG/0644/validación y refresco tras instalar | inline | `08d112f`, `999a422` | hecho |
| T3 | S4, S5, S6 | `extension.prepare`/`open`, status ampliado, CLI y página Navegadores | inline | `1187023` | hecho |
| T4 | S7, S8 | Orden de la lista, `focus_task`, `system.open`, `--show-task` y toast | inline + worker | `c0bfd60`, `763726b`, `1d15188` | hecho |
| T5 | S9 | Panel de calidades agrupado, default 720p y handoff | pendiente | — | pendiente |
| T6 | S10, S11 | Confirmación de captura y carpeta destino | pendiente | — | pendiente |
| T7 | todas | Gates, `.deb` 0.1.2 y QA en Ubuntu | pendiente | — | pendiente |

## Log

- L1 (pedido original del usuario, 12 nov, verbatim):

  Arrancamos Segmenta 0.1.2. Leé el estado actual y creá odd/tasks/segmenta-0.1.2.md. Corregí, en este orden, estos bloques con commits pequeños y gates por fase. No tag ni Release hasta que yo haga QA real en Ubuntu.

  P0 INSTALACIÓN/ESTADO DE EXTENSIÓN (hallazgos verificados):
  - La UI dice "Listo" sólo porque existe el manifiesto nativo; no comprueba la extensión. Cambiar copy y estado: "Manifiesto nativo listo" vs "Extensión no instalada/instalada/conectada". Nunca decir "Listo" sin calificar.
  - P0: browsers.rs usa FIREFOX_EXTENSION_ID="segmenta@wilkin.dev", pero el add-on real es "segmenta@wilkin.github.io". Corregir y atar con test.
  - Respetar XDG_CONFIG_HOME, chmod 0644, validar host, no sobrescribir silenciosamente; refrescar extension.status después de install-native y mostrar resultado por navegador.
  - Implementar extension.prepare (descomprimir ZIP a $XDG_DATA_HOME/segmenta/extensions/<family>-<version>, validar manifest/_locales/ID), extension.open (chrome://extensions, about:debugging, carpeta; argv fijo sin shell) y extension.status ampliado (system/user manifest, detección read-only de extensión en perfiles Chrome/Firefox, snap/Flatpak no soportado). Añadir CLI segmenta extension status|prepare|install-native|open y DemoClient. No escribir perfiles ni usar --load-extension en Chrome estable.
  - La página Navegadores debe ofrecer: estado real, "Preparar extensión", "Abrir página de extensiones", "Copiar ruta" y pasos claros; no decir instalada si no está.

  P0 ORDEN Y FOCO EN SEGMENTA:
  - DownloadStore::upsert hoy hace push: una tarea nueva queda al final. Insertar al inicio; reload debe conservar el orden created_at DESC del daemon. Tests.
  - Al añadir tarea desde extensión/CLI/GUI, traer Segmenta al frente y enfocar la tarea. Implementar focus_task (reset filtro/búsqueda, select, open_detail, present) y system.open/gui.activate o segmenta-gui --show-task <id>; la extensión debe llamarlo tras media.add/download.add. Notificación/toast de "Añadida a Segmenta" con carpeta.

  P1 PANEL DE CALIDADES:
  - media.probe devuelve formatos duplicados; agrupar/deduplicar. Opciones YouTube: 4K (2160), 1080p, 720p, 360p, Solo audio; sólo mostrar las disponibles y como máximo esas. Default 720p, no 4K. Elegir el mejor formato muxed/DASH por altura; audio por bitrate.
  - El botón Descargar debe crear la tarea y pasar la tarjeta a "Enviado a Segmenta", con [Abrir Segmenta]/[Ver progreso]; no dejar la lista interminable. media.add video debe usar height; no format_id; arreglar add_stream_task para respetar height.
  - Tests Node para grouping/default y handoff.

  P1 CAPTURA DE ARCHIVOS:
  - Para .rar/.zip/.deb/etc: NO cancelar la descarga antes de confirmar. Pausar (downloads.pause), abrir ventana propia de confirmación (windows.create/confirm.html) con nombre, host y carpeta destino; aceptar→cancelar+erase+download.add; rechazar/timeout→resume y no tocar host. Si el host falla, reanudar la descarga del navegador (nunca perder datos).
  - La ventana debe mostrar la carpeta efectiva: download.add/torrent.add deben devolver destination_dir/final_path; incluir en task_added.
  - Añadir setting capture_confirm (default true), ampliar capture_extensions con deb/rpm/appimage/xz/whl/jar/epub/vsix/xpi/msix, y actualizar tests/manifiestos/cache version.
  - Después de capturar, abrir/enfocar Segmenta y mostrar la tarea.

  QA que debés cubrir: Chrome for Testing y Firefox oficial; instalación preparada/verificada sin perfil real; estados de UI; orden arriba; foco; panel simplificado; handoff; confirmación de archivo y fallback; captura real local; no pérdida si host caído; gates completos y .deb 0.1.2. Reportá commits, SHAs y bloqueos. No publiques.

  ESTADO ACTUAL (12 nov, reanudar acá):
  - T1 1dbb310, T2 08d112f/999a422, T3 1187023 ya están commiteados; T4 c0bfd60 (upsert al inicio) y 763726b (focus_task) también.
  - Falta de T4: system.open en daemon con {task_id} que lance segmenta-gui --show-task <id> (argv fijo, sin shell), flag real en main.rs que cree cliente del daemon y llame focus_task tras download.list; toast 'Añadida a Segmenta' con final_path.
  - T5: media.probe/panel agrupar formatos en 4K/1080p/720p/360p + Solo audio, default 720p, sin format_id de video (usar height), tras Descargar mostrar tarjeta 'Enviado a Segmenta' con Abrir Segmenta/Ver progreso y llamar system.open; arreglar add_stream_task para height.
  - T6: archivos .zip/.rar/.deb/etc: downloads.pause, ventana confirm.html propia con nombre/host/carpeta, aceptar→cancelar+erase+download.add/torrent.add, rechazar/timeout→resume, host falla→resume sin perder archivo; destination_dir/final_path en respuesta/task_added; capture_confirm y extensiones deb/rpm/appimage/xz/whl/jar/epub/vsix/xpi/msix; cache version.
  - Todo con gates por paso y commits; no tag ni Release hasta QA en Ubuntu. Leé el ODD y el plan /home/wilkin/.commandcode/plans/segmenta-0.1.2.md si existe. No toques la release 0.1.1.

- L2: T1 corregido: `FIREFOX_EXTENSION_ID` era `segmenta@wilkin.dev` y el manifiesto declara `segmenta@wilkin.github.io`; nuevo test `firefox_extension_id_matches_the_shipped_manifest` lee el manifiesto real y falla si divergen.
- L3 (reconciliación 12 nov): el documento anterior marcaba T2/T3/T4 como pendientes pero `git log` muestra `08d112f`+`999a422` (T2), `1187023` (T3) y `c0bfd60`+`763726b` (T4 parcial). Se corrige la tabla; T4 sigue abierto por `system.open`, `--show-task` y el toast.
- L4: el plan `/home/wilkin/.commandcode/plans/segmenta-0.1.2.md` no existe en este equipo; la especificación vigente es este documento.
- L5: T4 partido en dos unidades. `1d15188` entrega el daemon (`system.open`, argv fijo sin shell, 1001 si la tarea no existe, `ERR_GUI_NOT_FOUND`) y la GUI (`--show-task` consumido antes de GLib, foco con reintento acotado). RED observado: con el brazo RPC renombrado, los dos tests nuevos fallan (-32601 en vez de éxito y de 1001).
- L6 (bloqueo de revisión nativa): el linaje `review-916de24d72c08cc8` (target `sha256:d648df92…`, base `7f470c5`, medio, 3 lentes) capturó `review-risk` y `review-readability`, pero `review-reliability` es imposible de enviar: el `subject-hash` que emite STATUS tiene 61 caracteres hex en lugar de 64, así que ningún binding reproducido byte a byte coincide («collectBinding is missing or stale for current STATUS»). No se reintenta a ciegas ni se inventa el valor; queda como decisión del usuario. La revisión nunca autoriza entrega y el QA en Ubuntu sigue siendo la puerta real.
