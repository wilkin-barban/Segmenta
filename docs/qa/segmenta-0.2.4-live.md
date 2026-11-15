# Segmenta 0.2.4 — QA live en Ubuntu

Fecha: 2026-11-15. Release `v0.2.4`. Estado del `.deb`:
`c9f9f8d958c25029e17b1e822b6af4d6a5551a5b1fcdec0993e841bfb0aedc1d`.
`check-package` 23/23 sobre `dist/segmenta_0.2.4-1_amd64.deb`.

## Qué se verificó

Prueba en vivo en el equipo del usuario (host `ssh ubuntu`, Ubuntu x86_64) con el paquete `0.2.4-1`
instalado y el daemon `0.2.4` en ejecución. Se verificaron los dos arreglos de la versión:

1. Alineación del ladder `qualities` del daemon a tiers nominales con un sondeo real de YouTube.
2. Presencia de la clave `contextMenuAddTorrent` en los tres catálogos de mensajes de la extensión
   preparada en disco.

No se usaron capturas de pantalla ni automatización de navegador en esta verificación.

## Sondeo real y ladder: antes vs después

Sondeo real ejecutado en el host contra `https://www.youtube.com/watch?v=FBhSudzU_ck`:

- **Formatos devueltos por el backend (`formats`)**: alturas reales
  `1184, 888, 592, 394, 296, 196, 128`.
- **Antes (v0.2.3 y anteriores)**: el daemon agrupaba `qualities` por altura exacta
  (`2160/1080/720/360`). Ninguna de las pistas coincidía exactamente, por lo que el ladder publicaba
  únicamente la entrada de audio (`left: ["audio"]`). La extensión tenía que reconstruir los tiers
  de video inspeccionando la lista cruda `formats`.
- **Después (v0.2.4)**: `media::quality_tier` y `QUALITY_TIERS` aplican los mismos cortes nominales
  que `extension/shared/protocol.js` (≥1800 → 4K, ≥900 → 1080, ≥600 → 720, ≥300 → 360). El ladder
  conserva la altura real de cada ganador y produce:
  ```
  1080@1184, 720@888, 360@592, audio
  ```
  y `default_quality` responde `720` comparando por tier (incluso con altura real 888). El daemon y
  la extensión coinciden plenamente.

## Localización del menú contextual (`contextMenuAddTorrent`)

- **Causa diagnosticada**: `extension/chromium/background.js` utilizaba
  `API.getMessage('contextMenuAddTorrent')`, pero dicha clave no existía en ningún archivo de
  `_locales`. En Chromium esto dejaba un `runtime.lastError` no capturado («Could not load the
  message») atribuido a `shared/browser-api.js:176` (`? i18n.getMessage(key)`).
- **Estado verificado en disco**: en la extensión preparada en
  `~/.local/share/segmenta/extensions/chromium-0.2.4`, la clave `contextMenuAddTorrent` está
  presente en los tres catálogos:
  - `_locales/en/messages.json`
  - `_locales/es/messages.json`
  - `_locales/pt_BR/messages.json`
  Chromium carga los mensajes sin registrar errores en consola ni en `runtime.lastError`.

## Estado instalado en el host de prueba

- `dpkg -s segmenta` → `Version: 0.2.4-1`.
- Daemon de Segmenta `0.2.4` activo bajo `segmenta.socket` y `segmenta.service`.
- Extensión preparada en `~/.local/share/segmenta/extensions/chromium-0.2.4` con `manifest.json` en
  `0.2.4` y los tres catálogos completos en `_locales`.

## Gates en el host de desarrollo

- `cargo test --workspace` → 408 tests.
- `node extension/tests/run.mjs` → 172 tests (incluye validación de claves en `background.js` y
  paridad en los tres idiomas).
- `cargo fmt --all -- --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `packaging/check-package.sh dist/segmenta_0.2.4-1_amd64.deb 0.2.4` → `paths=23/23`.

## Límite conocido

Desde el host de desarrollo (VPS con IP de datacenter) `media.probe` sobre `FBhSudzU_ck` responde
`1045 ERR_MEDIA_NOT_PUBLIC`. Por esa razón la verificación end-to-end en vivo se ejecutó en el host
del usuario (`ssh ubuntu`).
