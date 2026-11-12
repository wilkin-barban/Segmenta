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
- **S9.** El panel de calidades agrupa y deduplica formatos; opciones fijas 4K/1080p/720p/360p/Solo audio, sólo disponibles, default **720p**; el mejor muxed/DASH por altura y audio por bitrate. Descargar crea la tarea y pasa la tarjeta a "Enviado a Segmenta" con [Abrir Segmenta]/[Ver progreso]. `media.add` video usa `height`, no `format_id`; `add_stream_task` respeta `height`.
- **S10.** Captura de archivos: **no** se cancela antes de confirmar. Pausar (`downloads.pause`), abrir `confirm.html` (`windows.create`) con nombre, host y carpeta destino; aceptar → cancelar + erase + `download.add`; rechazar/timeout → `resume`. Si el host falla, se reanuda la descarga del navegador.
- **S11.** `download.add`/`torrent.add` devuelven `destination_dir`/`final_path` e informan en `task_added`; setting `capture_confirm` (default true) y extensiones ampliadas (deb, rpm, appimage, xz, whl, jar, epub, vsix, xpi, msix).

## Tasks

| ID | Spec | Entrega | Estado |
| --- | --- | --- | --- |
| T1 | S2 | ID de Firefox corregido y atado al manifiesto | hecho (`amendment`) |
| T2 | S1, S3, S6 | Estado real por navegador, XDG/0644/validación y refresco tras instalar | pendiente |
| T3 | S4, S5, S6 | `extension.prepare`/`open`, status ampliado, CLI y página Navegadores | pendiente |
| T4 | S7, S8 | Orden de la lista, `focus_task` y `system.open` | pendiente |
| T5 | S9 | Panel de calidades agrupado, default 720p y handoff | pendiente |
| T6 | S10, S11 | Confirmación de captura y carpeta destino | pendiente |
| T7 | todas | Gates, `.deb` 0.1.2 y QA en Ubuntu | pendiente |

## Log

- L1: pedido del usuario para 0.1.2 (P0 instalación/estado, P0 orden/foco, P1 panel de calidades, P1 captura con confirmación; sin tag ni Release).
- L2: T1 corregido: `FIREFOX_EXTENSION_ID` era `segmenta@wilkin.dev` y el manifiesto declara `segmenta@wilkin.github.io`; nuevo test `firefox_extension_id_matches_the_shipped_manifest` lee el manifiesto real y falla si divergen.
