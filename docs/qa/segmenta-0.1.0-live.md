# Segmenta 0.1.0 — tablero vivo

## Incidencia de empaquetado de la extensión (Chrome for Testing)

Chrome rechazó el ZIP con `Default locale was specified, but _locales subtree is
missing`. Causa: `manifest.json` declara `default_locale: es` y los catálogos viven en
`extension/_locales/`, fuera de `extension/chromium` y `extension/firefox`; el empaquetado
copiaba sólo el contenido del target.

Fix:

- `extension/tools/pack.mjs` copia `_locales` dentro de cada target (para que también
  funcione `--load-extension=extension/chromium`) y **valida antes de comprimir**:
  `default_locale` presente en `_locales`, los tres catálogos con el mismo conjunto de
  claves y cada `__MSG_*__` resuelto.
- ZIP propio (sin dependencias) con fecha fija, para que el asset sea reproducible.
- `extension/tests/pack.test.mjs` (6 tests) y encadenado en `extension/tests/run.mjs`.

Verificación: `unzip -l` muestra `_locales/{es,en,pt_BR}/messages.json` en ambos ZIP;
`node extension/tests/run.mjs` pasa con los 6 tests nuevos; el `.deb` no incluye assets
de la extensión, así que su SHA no cambia.

## Hallazgos del QA en vivo (0.1.0)

| # | Hallazgo | Estado |
| --- | --- | --- |
| 1 | Panel: `ERR_HOST_PROTOCOL · reply is not a JSON-RPC object` por auto-resolve por aridad en `invoke()` y una Promise nativa que resuelve `undefined` | **Corregido** (`fbb6715`): espera el callback y descarta la Promise vacía; test con API callback + Promise que resuelve `undefined`; 67 tests Node |
| 2 | ZIP sin `_locales` (Chrome lo rechaza) | **Corregido**: `tools/pack.mjs` copia/valida y ambos ZIP llevan es/en/pt_BR |
| 3 | `.deb` sin CLI de medios visible en `--help` | **Corregido** (`18d7e36`): `media probe|add` documentados en es/en/pt-BR; `.deb` reconstruido |
| 4 | Progreso de medios en 0 (bytes/total/speed) | **Corregido**: el runner lee también **stderr** (donde yt-dlp emite `--progress-template`); test con el fake emitiendo por stderr y aserción de `downloaded_bytes > 0` |
| 5 | 720p falla con 403 y, con `yt-dlp` actual, el selector muxed antiguo no existe | **Corregido**: selector `b[height<=H]/18/bv*[height<=H]+ba/b[height<=H]/best`; verificado con `398+251` y descarga real de 81.554.068 B (AV1 720p + Opus) |
| 6 | HLS/DASH no cableado (`1043`) | **Corregido**: `media.probe/add` resuelven manifiestos con `segmenta-core::stream`, descargan fragmentos con progreso y remuxean con `ffmpeg -c copy`; test e2e con fixture HLS generado por ffmpeg y servidor local |

## Segunda ronda de QA en vivo (Chrome for Testing, `--mute-audio`)

| # | Hallazgo | Estado |
| --- | --- | --- |
| 7 | `-f "b[height<=720]/18/best"` → `ERROR: Requested format is not available`; sólo hay DASH video-only + audio | **Corregido**: el selector es `b[height<=H]/18/bv*[height<=H]+ba/b[height<=H]/best`; verificado con el `yt-dlp` 2026.10.26 del usuario (`398+251`) |
| 8 | Destino existente + `--continue`/`--no-part` → HTTP 416 | **Corregido**: cada tarea reserva un nombre único (`resolve_unique_path`) y el runner usa `--no-continue` |
| 9 | Tareas YouTube completadas con `downloaded_bytes=0` y panel al 0 % | **Corregido**: sin throttle en el callback y, al completar, el tamaño real del archivo/directorio es la fuente de verdad (`output_size`) |
| 10 | DASH `SegmentList`/`SegmentBase` rechazado (correcto) | **Limitación documentada**: 0.1.0 soporta DASH con `SegmentTemplate`; `SegmentList`/`SegmentBase` quedan fuera con `1043` |
| 11 | `yt-dlp` 2026.03 del sistema devuelve 403/formatos viejos | **Validación añadida**: `media.probe/add` comparan `yt-dlp --version` con `MIN_YTDLP_VERSION` (2026.10.01) y fallan con `1044` y un mensaje accionable |

## Cierre de QA 0.1.0 (tercera ronda)

| # | Verificación | Resultado |
| --- | --- | --- |
| 12 | Probe de playlist con una entrada oculta posterior | **Corregido** (`409b036`): `--playlist-end N` se aplica antes de `-J`; límite compartido entre probe y descarga |
| 13 | Playlist pública con límite 2 | **OK**: `PL590…` completó 93.677.936 B en 2 MP4 1080p VP9+Opus; `PLbpi…` completó 1.220.442.162 B en 2 MP4 4K/1080p |
| 14 | Repetición de audio con destino existente | **OK**: creó `… (1).m4a` y completó; sin 416 |
| 15 | HLS público | **OK**: `x36xhzz.m3u8` completó 488.196.902 B, MP4 1080p H264+AAC |
| 16 | Chrome for Testing | **OK**: ZIP con `_locales`, panel con 24 calidades, audio desde el panel completado |
| 17 | Firefox | **OK**: Firefox 160 oficial, extensión temporal `segmenta@wilkin.github.io`, panel y descarga de audio completados desde el panel. Firefox snap 157 no permite la conexión remota de `web-ext`; limitación de automatización, no del add-on |
| 18 | DASH | Probe de `bbb_30fps.mpd` con SegmentTemplate OK; `SegmentList/SegmentBase` sigue fuera de alcance con `1043` (documentado) |
| 19 | Gates finales | `cargo test --workspace` 45 suites, GUI Xvfb OK, 67 tests de extensión, `clippy -D warnings` 0, `fmt --check` OK, `check-package.sh` 18/18 |

Artefactos finales:

- `segmenta_0.1.0-1_amd64.deb` — 4.167.904 B — `a8c62a4a66623357468329fd16a0c55c98a08e016f88ba92a7c0ad2006c7bd99`
- `segmenta-extension-0.1.0.zip` — 35.413 B — `137f95b38769d32235dece579366ab14b1241aa20302ae919ab717b8b6d62626`
- `segmenta-firefox-0.1.0.zip` — 35.127 B — `8242c92381a459e08353bcef79bd0682ba25d41d9087a7e7770c6c7b355270d9`

Requisito operativo: `yt-dlp >= 2026.10.01` (el 2026.03 de Ubuntu devuelve 403) y `ffmpeg`; el host nativo y los siete manifiestos viajan en el `.deb`.
