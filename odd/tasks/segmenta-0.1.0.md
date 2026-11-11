# Segmenta 0.1.0 — media, extensión Chromium+Firefox y host nativo

Release `0.1.0`: extensión de navegador para YouTube (familia Chromium completa y Firefox), host de
Native Messaging, backend de medios con `yt-dlp` y HLS/DASH públicos sin cifrar. Documento ODD; se lee
hasta `## Log`. Sin publicar hasta el QA remoto del usuario.

## Specs

### S1 — Host nativo común (Chromium y Firefox)
`crates/segmenta-host` implementa el framing de Native Messaging: 4 bytes little-endian con el largo +
JSON UTF-8, máximo 1 MiB por mensaje; bytes extra tras un mensaje completo son `ERR_FRAME_TRAILING`;
EOF a mitad de cabecera/mensaje es `ERR_FRAME_TRUNCATED`; el proceso termina con código 0. Conecta al
socket del daemon y reintenta una vez arrancando `segmenta.socket` si no está activo. Una solicitud →
una respuesta correlacionada por `id`; no usa shell, no acepta rutas ni comandos del navegador y
rechaza métodos que no empiecen por `media.`/`system.`/`download.` conocidos.

### S2 — Extensión Chromium MV3
`extension/chromium/` con `manifest.json` MV3: `nativeMessaging`, `storage`, `scripting`, `activeTab`,
`host_permissions` de `https://www.youtube.com/*`, `key` fija de desarrollo y `content_scripts`
(`document.js` en `world: MAIN`, `content.js` en `ISOLATED`). El ID de extensión se deriva de la `key`
y debe coincidir con el `allowed_origins` del manifiesto nativo; hay un test que compara ambos.

### S3 — Extensión Firefox
`extension/firefox/` con `manifest.json` WebExtension, `browser_specific_settings.gecko.id`
`segmenta@wilkin.github.io`, `permissions` mínimos y `nativeMessaging`. El manifiesto nativo de Firefox
usa `allowed_extensions: ["segmenta@wilkin.github.io"]`. `extension/shared/browser-api.js` expone
`browser.*` con respaldo `chrome.*` para background/content/popup.

### S4 — Botón flotante y panel en YouTube
Sobre `youtube.com/watch` y `youtube.com/playlist`, `content.js` inyecta un botón flotante
«Descargar con Segmenta» aislado con Shadow DOM (sin heredar CSS del sitio) y un panel con: calidad de
video devuelta por el daemon, «Solo audio», «Video actual» vs «Playlist completa (N)», límite de
playlist (default 100, editable) y estado/progreso. Sólo actúa tras un gesto explícito del usuario;
valida que la URL sea `https://` y de un host permitido antes de pedir nada.

### S5 — Seguridad de la extensión
El `background` sólo acepta mensajes de la propia extensión y del content script; valida el esquema,
el host y el tamaño de los parámetros, y nunca ejecuta comandos ni rutas. `document.js` entrega los
eventos de `fetch`/XHR (`m3u8`/`mpd`) por `MessageChannel` validando el origen; no usa
`postMessage("*")` sin comprobar. No se leen cookies ni contenido privado; no hay bypass de DRM.

### S6 — `media.probe`
Nuevo método JSON-RPC `media.probe` con `{url, kind: "video"|"playlist"|"stream", limit?}`. Para
YouTube ejecuta `yt-dlp --no-config --no-warnings -J <url>` con argumentos separados (sin shell) y
devuelve `{kind, title, duration_seconds, thumbnail, formats: [{format_id, label, height, ext, kind}],
audio: [...], playlist: {count, entries[]}}`. Para HLS/DASH descarga y parsea el manifiesto y devuelve
variantes/calidades y pistas. URL inválida, host no permitido, contenido privado o DRM devuelven error
con código estable.

### S7 — `media.add`
`{url, mode: "video"|"audio"|"playlist", format_id?, height?, playlist_limit?, kind?}` crea una tarea
con `download_type` (`youtube_video`, `youtube_playlist`, `youtube_audio`, `hls`, `dash`) y devuelve
`{task_id, status}`. El límite de playlist se acota a 1–500 (default 100). Una playlist crea una tarea
agregada con `playlist_count` y un `batch_id`; cada entrada reporta progreso agregado.

### S8 — Backend `yt-dlp`
El daemon ejecuta `yt-dlp` sin shell: `--no-config`, `--newline`, `--progress-template` para parsear
porcentaje/velocidad/ETA, `-f` con los formatos devueltos por el probe, `--merge-output-format mp4`,
`--continue`, `--playlist-end <limit>` en playlists y `bestaudio/best` sin recodificar en audio. La
pausa o cancelación termina el proceso hijo; un error del proceso produce `task_failed` con código
`ERR_MEDIA_BACKEND` y el último mensaje legible. Nunca se pasa la URL por shell ni se escribe en logs
con query string.

### S9 — HLS/DASH públicos sin cifrar
`segmenta-core/src/stream/` parsea M3U8 (master, variantes, `EXT-X-MAP`, `EXTINF`,
discontinuidades) y MPD (`SegmentTemplate`, `SegmentTimeline`, `Initialization`, audio/video), rechaza
`EXT-X-KEY` distinto de `NONE` y `ContentProtection` de DRM con `ERR_DRM_PROTECTED`, descarga los
fragmentos con el scheduler existente, escribe el segmento de inicialización primero y une con
`ffmpeg -c copy` invocando el binario con argumentos separados (sin shell). Sin live, AES-128,
SAMPLE-AES, subtítulos ni multi-audio.

### S10 — Persistencia de medios
Migración SQLite idempotente (`user_version` 2): `media_kind`, `media_format`, `playlist_title`,
`playlist_count`, `batch_id` en `downloads`, con índices para `batch_id`. `download_type` ya admite
`hls`/`dash`; se agregan los tipos de YouTube. Un daemon que arranca con la versión 1 migra sin perder
filas.

### S11 — Eventos y `media.status`
`media.status` acepta `{task_id}` o `{batch_id}` y devuelve estado, progreso, archivo final, título,
tipo y error. Los eventos `download.event` de una tarea de medios incluyen `media_kind` y
`playlist_progress {completed, total}` para la extensión y la GUI; `seq_id` sigue siendo monótono.

### S12 — GUI
La lista y el detalle muestran tareas de medios con tipo, título, calidad/audio y progreso de playlist;
los filtros, el buscador y la matriz de acciones existentes siguen funcionando (tests ampliados). No se
agregan controles decorativos.

### S13 — CLI
`segmenta media probe <url> [--kind video|playlist|stream] [--limit N]` y
`segmenta media add <url> --mode video|audio|playlist [--format-id ID] [--height H] [--limit N]`, con
salida humana localizada es/pt-BR/en y `--json`; no rompe los comandos existentes.

### S14 — Matriz de navegadores e instalación nativa
`extension/install-native.sh` detecta los navegadores instalados (Chrome, Chromium, Brave, Edge,
Vivaldi, Opera, Firefox) y escribe su manifiesto nativo de usuario apuntando a `segmenta-host`. El
`.deb` instala los manifiestos de sistema de los soportados y documenta el paso por navegador.
Navegadores soportados con manifiesto: Chrome, Chromium, Brave, Edge, Vivaldi, Opera y Firefox.

### S15 — Empaquetado 0.1.0
Versión `0.1.0` en workspace y binarios; nuevo crate `segmenta-host` en `/usr/bin/segmenta-host`;
`Depends: libsecret-tools, ffmpeg, yt-dlp`; el `.deb` instala los manifiestos de sistema; se generan
`dist/segmenta-extension-0.1.0.zip` (Chromium), `dist/segmenta-firefox-0.1.0.zip` (XPI) y
`dist/segmenta_0.1.0-1_amd64.deb` + `.sha256`. `check-package.sh` valida los binarios, los
manifiestos, el ZIP y el XPI.

### S16 — Límites y errores explícitos
Privados, DRM, live, AES-128/SAMPLE-AES, multi-audio y subtítulos se rechazan con códigos estables y
mensajes localizados; `yt-dlp` ausente o desactualizado produce `ERR_MEDIA_BACKEND` con la instrucción
de actualizar, nunca un falso éxito.

### S17 — i18n de la extensión
`extension/_locales/{es,en,pt_BR}/messages.json` cubre todo el texto del panel, botón, errores y
tooltips; test de paridad de claves entre los tres catálogos.

### S18 — Puertas de calidad
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `xvfb-run -a cargo test -p segmenta-gui`, tests de la extensión (Node) y
`packaging/check-package.sh` sobre el `.deb`.

### S19 — QA remoto con Chrome y Firefox
El usuario prueba en `ssh ubuntu` con **Chrome 156 y Firefox 157** (ambos instalados): carga de la
extensión, botón flotante, panel de calidades, video, playlist con límite, solo audio, HLS/DASH público,
rechazo de DRM/privado, errores, capturas y GUI. Chromium/Brave/Edge/Vivaldi/Opera: manifiestos y smoke
sólo si están instalados; no se inventan resultados.

### S20 — Documentación
`README.md`, `docs/openspec/03` (flujo YouTube, formatos, playlist, audio, navegadores),
`07` (`media.*`, errores, migración), `09` (permiso mínimo, ID de desarrollo, sin cookies ni DRM) y
`10` (host, manifiestos, ZIP/XPI, `yt-dlp`/`ffmpeg`) actualizados; `docs/qa/segmenta-0.1.0-live.md`
como tablero. Nota de alcance: público, sin DRM, sin privados; no prometer «cualquier YouTube».

### S21 — Higiene
Sin secretos, sin `target/`, `.env` ni `.atl/` versionados; el tag `v0.0.2` y su Release no se tocan;
`0.1.0` no se publica hasta el informe verde del usuario.

## Tasks

| ID | Specs | Tarea | Rutas | Test RED |
| --- | --- | --- | --- | --- |
| T1 | S1 | Crate `segmenta-host`: framing, límites, socket, retry | `crates/segmenta-host/**` | `frame_roundtrip`, `rejects_oversized_and_truncated`, `connects_and_proxies` |
| T2 | S2-S5, S17 | Extensión Chromium + compartidos (button, panel, audio, playlist, i18n) | `extension/shared/**`, `extension/chromium/**` | `message_validation`, `locale_parity` |
| T3 | S3, S5 | Extensión Firefox y adaptador `browser.*` | `extension/firefox/**`, `extension/shared/browser-api.js` | `gecko_manifest`, `adapter_calls` |
| T4 | S14 | Script de instalación nativa multi-navegador + manifiestos | `extension/install-native.sh`, `packaging/native-messaging/**` | `writes_every_installed_browser` |
| T5 | S6-S8, S10-S11 | Daemon media: probe/add/status, yt-dlp, migración, eventos | `crates/segmenta-daemon/src/media/**`, `store.rs`, `manager.rs`, `rpc.rs` | `media_probe_parses_formats`, `media_add_playlist_limit`, `migration_v2_idempotent` |
| T6 | S9 | Parsers HLS/DASH, rechazo DRM, merge con ffmpeg | `crates/segmenta-core/src/stream/**`, `tests/stream/**` | `m3u8_variants`, `mpd_template`, `drm_rejected`, `merge_plan` |
| T7 | S12-S13 | GUI/CLI de medios | `crates/segmenta-gui/src/**`, `crates/segmenta-cli/src/**` | `media_rows_and_labels`, `media_cli_output` |
| T8 | S15 | Empaquetado 0.1.0, ZIP/XPI, check-package | `Cargo.toml`, `packaging/**`, `crates/segmenta-gui/Cargo.toml` | `check_package_lists_host_and_manifests` |
| T9 | S20 | Documentación y tablero | `README.md`, `docs/openspec/*`, `docs/qa/**` | — |
| T10 | S18-S19 | Gates y candidato para QA | `dist/**` | comandos de S18 |

## Riesgos y mitigación

- **YouTube cambia**: `yt-dlp` del sistema puede quedar viejo. El daemon informa `ERR_MEDIA_BACKEND`
  con «actualizá yt-dlp»; nunca finge éxito. Se documenta la actualización por el sistema.
- **Navegadores**: el ID de la extensión debe coincidir con el manifiesto nativo; test de comparación
  `key` ↔ `allowed_origins`. Firefox sin firma se carga temporalmente; se documenta.
- **Playlists enormes**: límite default 100, máximo 500, cancelable.
- **HLS/DASH**: sólo público y sin cifrar; DRM y claves se rechazan con código explícito.
- **Contenido privado/edad**: fuera de alcance; el probe devuelve un error claro.
- **Entorno**: sin `yt-dlp` local los tests usan un binario falso; la prueba real es del usuario en el
  remoto con `yt-dlp` instalado por el `.deb`.

## Criterios de aceptación

Los del plan más: host con framing correcto y round-trip contra el daemon; extensión Chromium y
Firefox cargables con ID de desarrollo coincidente; manifiestos nativos instalados por el `.deb` y el
script; `media.probe/add/status` funcionando con `yt-dlp` real en el remoto; HLS/DASH público sin
cifrar con merge; DRM/privado/ live rechazados; GUI/CLI mostrando tareas de medios; `.deb`, ZIP y XPI
en `dist/` con checksums; gates verdes. **No se publica nada hasta el informe verde del usuario.**

## Log

**L1 (pedido de arranque, verbatim):**

> Comenzamos **Segmenta 0.1.0**. El plan aprobado está en:
> `/home/wilkin/.commandcode/plans/segmenta-0.1.0-browser-extension.md`
>
> Leelo completo antes de editar. Estás en `main`, sin ramas ni worktrees. Todo se commitea en unidades atómicas y se publica en `main`.
>
> ## Objetivo
>
> Extensión Chrome MV3 para YouTube con botón flotante, selector visible de calidades, “solo audio” y descarga de playlist completa con límite; Native Messaging; backend de medios en el daemon con `yt-dlp`; HLS/DASH públicos sin cifrar; integración con GUI/CLI; paquete 0.1.0 y ZIP de extensión. El QA en vivo en `ssh ubuntu` con Chrome/YouTube lo haré yo; no publiques nada hasta que te devuelva el informe verde.
>
> ## Decisiones cerradas
>
> - Dependencias del sistema: `yt-dlp` y `ffmpeg` declarados por el `.deb`; no embebas binarios.
> - Playlist: toda la lista pública, límite configurable default 100, cancelable.
> - Alcance: YouTube + HLS/DASH públicos **sin cifrar**. DRM, AES-128/SAMPLE-AES, privados, live, subtítulos y multi-audio fuera.
> - Audio: mejor audio original (M4A/Opus), sin recodificar por defecto.
> - Chrome: extensión en modo desarrollador/ZIP, sin Chrome Web Store. Firefox sólo manifiesto/documentación, sin QA prioritario.
> - Pruebas vivas: videos públicos de muestra; no dependas de URLs del usuario.
> - No toques código vendor de gentle-shell.
>
> ## Fases obligatorias
>
> 1. **ODD + contratos**: antes de código, creá `odd/tasks/segmenta-0.1.0.md` con specs, tareas TDD, riesgos y criterios. Actualizá `docs/openspec/03`, `07`, `09`, `10` y `README.md` con el contrato de medios, `media.probe/add/status`, formatos, playlist, audio, host nativo, manifiestos, `yt-dlp`/`ffmpeg`, ZIP y límites. Commiteá el ODD solo.
> 2. **`crates/segmenta-host`**: framing LE 32-bit, límite 1 MiB, socket, arranque/retry del socket, tests de framing/errores/round-trip; sin shell.
> 3. **Extensión Chrome**: `extension/` con manifest MV3, key/ID determinista, background, content/Shadow DOM, botón flotante en YouTube, panel de calidades/audio/playlist, validación de gesto/URL, i18n es/en/pt-BR, iconos. Tests de mensajes/validación.
> 4. **Daemon media**: `media.probe`, `media.add`, `media.status`; backend `yt-dlp` para YouTube (video/playlist/audio, args seguros, progreso, pausa/cancelación, errores claros); tasks en SQLite con tipo de medio; eventos para GUI/extensión; migración idempotente.
> 5. **HLS/DASH**: parser público sin cifrar, init segment, selección de calidad, descarga concurrente, unión con FFmpeg sin shell, rechazo explícito de DRM/AES; fixtures locales y tests.
> 6. **GUI/CLI**: tareas de medios visibles, filtros y apertura de carpeta; `segmenta media probe/add`; no romper 0.0.2.
> 7. **Empaquetado**: versión 0.1.0, `segmenta-host`, manifiestos nativos Chrome/Firefox, `Depends: yt-dlp, ffmpeg`, check-package actualizado, ZIP `segmenta-extension-0.1.0.zip`.
> 8. **Gates**: fmt, clippy -D warnings, cargo test workspace, GUI Xvfb, check-package. Generá `.deb` y `.sha256` y reportá ruta/tamaño/SHA.
>
> ## Reglas
>
> - TDD estricta: tests RED antes de código.
> - No publiques, no tag, no Release, no toques `v0.0.2`.
> - El QA remoto lo hago yo: Chrome, YouTube, video, playlist, audio, HLS/DASH, capturas, errores.
> - Si YouTube/Chrome requiere decisiones o falla, reportá el bloqueo real; no inventes.
> - Mantené el TODO visible y actualizado con `todo`.
> - Usá scratch temporal; evitá `rm -rf` de rutas ajenas.
> - Si el alcance se vuelve inmanejable, priorizá: host + extensión + YouTube video/playlist/audio + paquete; HLS/DASH después, pero no publiques 0.1.0 sin lo que el plan exige.
>
> Empezá por el ODD y la exploración del código; después implementá. Reportá commits, tests y candidato para QA.

**L2 — Corrección de alcance (verbatim):**

> Corrección de alcance obligatoria para 0.1.0: **no es una release sólo-Chrome**.
>
> Soportá la familia Chromium completa y Firefox:
>
> - Chromium/Chrome/Brave/Edge/Vivaldi/Opera: misma extensión MV3 con `key`/ID estable y manifiestos nativos en las rutas de cada navegador.
> - Firefox: target WebExtension separado con `browser_specific_settings.gecko.id` y manifiesto `allowed_extensions`; adaptador `browser.*`/`chrome.*`.
> - `segmenta-host` es común; un script de instalación de desarrollo debe detectar navegadores y escribir sus manifiestos. El `.deb` instala los manifiestos de sistema de los navegadores soportados.
> - Assets de release: ZIP para Chromium, XPI/ZIP para Firefox y `.deb`.
> - Pruebas en vivo obligatorias: Chrome y Firefox en `ssh ubuntu` (Chrome 156 y Firefox 157 ya están instalados). Chromium/Brave/Edge/Vivaldi/Opera: validar manifiestos y smoke si están instalados; no inventar pruebas de navegadores ausentes.
> - Firefox sin firma de tienda se carga temporalmente desde `about:debugging`; documentá la limitación.
> - Actualizá el plan, el ODD, docs 03/09/10, README y el TODO para reflejar la matriz. No reduzcas el alcance a Chrome.
>
> Seguí con la implementación; el QA remoto lo hago yo con Chrome + Firefox.

**L3 — Estado inicial.** `main` limpio, `v0.0.2` y su Release ya publicados; el plan aprobado se leyó
completo y se actualizó con la matriz de navegadores. Sin `yt-dlp` local: los tests de medios usarán un
binario falso y la prueba real queda para el remoto.
