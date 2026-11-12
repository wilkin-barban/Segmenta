# Segmenta 0.1.1

Release de integración con navegadores, captura de descargas y torrents.

## Instalación de la extensión

- Detección de Chrome, Chromium, Brave, Edge, Vivaldi, Opera y Firefox.
- Página **Navegadores** con estado del manifiesto nativo, instalación por usuario, enlace de tienda cuando exista y fallback local.
- Chrome estable puede requerir **Cargar descomprimida** una vez; Firefox oficial acepta carga temporal.
- Los manifiestos nativos se instalan en el `.deb`; Firefox snap y Flatpak se marcan como no soportados.

## Descargas del navegador

- `downloads.onCreated` captura archivos por extensión, con denylist y cancelación de la descarga nativa.
- Lista por defecto de 54 extensiones, incluyendo `3gp`, `7z`, `aac`, `ace`, `apk`, `arj`, `iso`, `r0*`, `r1*`, `rar`, `sit`, `sitx`, `tif`, `wmv` y `pkg`.
- Lista editable en Preferencias; `r0*`/`r1*` se interpretan como prefijos.
- `.torrent` se deriva a `torrent.add`; los enlaces `magnet:` se ofrecen en el menú contextual y mediante un handler `.desktop`.

## Torrents

- Motor embebido `librqbit 9.1`; una sesión compartida por proceso.
- Schema v3 con tablas `torrents` y `torrent_files`.
- RPC/CLI `torrent probe|add|status`, progreso, cancelación y seeding desactivado por defecto.
- Saneamiento de rutas internas (`..`, absolutas, escapes) y tests hostiles.

## Correcciones de UI

- El hover sobre una tarea ya no abre el detalle; sólo un clic real lo abre.
- “Atrás” limpia la selección y permite reabrir la tarea.
- Badge específico para torrents.

## Requisitos

- Ubuntu 26.04, `ffmpeg` y `yt-dlp >= 2026.10.01`.
- Chrome/Chromium/Edge/Brave/Vivaldi/Opera o Firefox oficial.
- Firefox snap y Flatpak no pueden ejecutar el host nativo.

## Verificación

- 46 suites Rust, GUI bajo Xvfb, 76 tests de extensión, clippy `-D warnings` 0, `packaging/check-package.sh` 22/22.
- QA en Ubuntu: captura `.zip` real, motor torrent real de Sintel, magnet/.torrent, hover, smoke GUI.
- Limitaciones documentadas: cold-start del service worker, probe de `.torrent` multiarchivo y registro automático del handler del sistema según escritorio.

## Artefactos

- `segmenta_0.1.1-1_amd64.deb` — 6.8 MB — `a62123f795eb3855f8e4c676eb6f8395767d7f84d0fc5797f53a235c3ce7a450`
- `segmenta-extension-0.1.1.zip` — `342980b7cd33e1e5e9a48788385c2bc8705d619db4a59436eb81773ac6e08ee6`
- `segmenta-firefox-0.1.1.zip` — `95446187c127b0ec496ebf3e5f727816ebdafe22e36c2f1e0cba210f91040233`
- `segmenta_0.1.1-1_amd64.deb.sha256`
