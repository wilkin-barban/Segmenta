# Segmenta 0.2.3 — QA live en Ubuntu

Fecha: 2026-11-15. Release `v0.2.3`. Estado del `.deb`:
`4c9726c88ab160671d9dffb61e8a3ffb3b83955546f9ef67f7404b117359d4a0`.
`check-package` 23/23 sobre `dist/segmenta_0.2.3-1_amd64.deb`.

## Qué se verificó

Prueba en vivo de la extensión `0.2.3` sobre YouTube, hecha por el usuario en su equipo Ubuntu
26.04 (host `ssh ubuntu`, x86_64). Resultado observado: el panel **ofrece opciones de descarga** en
el video del reporte y **la descarga se completa**. No hubo capturas simuladas ni automatización de
navegador en esta verificación.

## Causa confirmada y corregida

`media.probe --kind video` de `FBhSudzU_ck` devuelve formatos con alturas reales
`1184, 888, 592, 394, 296, 196, 128` y un ladder `qualities` que sólo trae audio. La extensión
anterior agrupaba por altura exacta (`2160/1080/720/360`), no encontraba ninguna coincidencia y caía
al fallback de «Solo audio». Con tiers nominales el mismo payload produce `1080p`, `720p`, `360p` y
«Solo audio», con 720p preseleccionado.

Reproducción del payload exacto (evaluado sobre la copia **preparada** en disco, no sobre el árbol):

```
1080@1184, 720@888, 360@592, audio | default 720
```

## Estado instalado en el host de prueba

- `dpkg -s segmenta` → `Version: 0.2.3-1` (venía de `0.2.2-1`).
- `segmenta-cli hello` → `daemon de segmenta 0.2.3 (protocolo v1)`; `segmenta.socket` y
  `segmenta.service` activos.
- `segmenta-cli extension prepare --family all` →
  `/home/wilkin/.local/share/segmenta/extensions/chromium-0.2.3` y `…/firefox-0.2.3`.
- `extension.status` → `package 0.2.3`; `chromium 0.2.3 / 0.2.3 / match`.
- Carpeta preparada: 19 archivos, `manifest.json` con `0.2.3` y `_locales` presente;
  `sha256(shared/protocol.js)` = `619c1dbabe0dd12efa81afed96a8b65b17abb4614ea04e3c052854a97a1ec3ad`
  y `sha256(manifest.json)` =
  `780778f11b160be1c0cb05d9e98f425ffa24099a48b7a47364e0600b01358a36`, idénticos al árbol de
  desarrollo.
- No había descargas activas al reiniciar las unidades de usuario.

## Gates en el host de desarrollo

- `node extension/tests/run.mjs` → 172 tests.
- `cargo test --workspace` → 407 tests; `xvfb-run -a cargo test -p segmenta-gui` → 95 tests.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- `node extension/tools/sync-shared.mjs --check`, `node extension/tools/pack.mjs --check`.
- `segmenta-gui --smoke` → `SMOKE OK rows=1 segments=4`.

## Límite conocido

Desde el host de desarrollo (VPS con IP de datacenter) `media.probe` de `FBhSudzU_ck` responde
`1045 ERR_MEDIA_NOT_PUBLIC`, así que la prueba end-to-end sólo es posible en el equipo del usuario.
La evidencia de arriba es la de esa prueba más la reproducción del payload exacto en los tests.
