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
| 1 | Panel: `ERR_HOST_PROTOCOL · reply is not a JSON-RPC object` por auto-resolve por aridad en `invoke()` | **Corregido** (`b9756a2`/`f35ca55`): espera el callback; test con API de aridad 2 que responde async; 66 tests Node |
| 2 | ZIP sin `_locales` (Chrome lo rechaza) | **Corregido**: `tools/pack.mjs` copia/valida y ambos ZIP llevan es/en/pt_BR |
| 3 | `.deb` sin CLI de medios visible en `--help` | **Corregido** (`18d7e36`): `media probe|add` documentados en es/en/pt-BR; `.deb` reconstruido |
| 4 | Progreso de medios en 0 (bytes/total/speed) | **Corregido**: el runner lee también **stderr** (donde yt-dlp emite `--progress-template`); test con el fake emitiendo por stderr y aserción de `downloaded_bytes > 0` |
| 5 | 720p falla con 403 | **Mitigado**: modo video usa selector **muxed** `b[height<=H]/18/...` en vez del `format_id` video-only; test unitario del argv |
| 6 | HLS/DASH no cableado (`1043`) | **Corregido**: `media.probe/add` resuelven manifiestos con `segmenta-core::stream`, descargan fragmentos con progreso y remuxean con `ffmpeg -c copy`; test e2e con fixture HLS generado por ffmpeg y servidor local |
