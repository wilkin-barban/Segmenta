<p align="center">
  <img src="https://raw.githubusercontent.com/wilkin-barban/Segmenta/main/packaging/io.github.wilkin.Segmenta.svg" width="96" alt="Icono de Segmenta">
</p>

# Segmenta 0.2.4

El daemon y la extensión vuelven a coincidir en cómo agrupan las calidades, y la extensión deja de
reportar un error de localización en Chrome.

## Qué problema resuelve

La 0.2.3 arregló el panel agrupando por tier nominal, pero el ladder `qualities` del daemon seguía
agrupando por altura exacta. Para un video cuyas pistas miden `1184, 888, 592, 394, 296, 196, 128` el
daemon publicaba un ladder con **sólo audio**: la extensión podía reconstruirlo desde la lista cruda
`formats`, pero cualquier consumidor del ladder (CLI incluida) veía una escalera vacía.

En paralelo, el menú contextual de torrents usaba `API.getMessage('contextMenuAddTorrent')` y esa
clave no existía en ningún catálogo. Chrome registraba un `lastError` sin leer («Could not load the
message») y lo señalaba en `shared/browser-api.js:176`, la línea de `i18n.getMessage`.

## Cambios

### Ladder del daemon por tier nominal

`media::quality_tier` aplica los mismos cortes que `extension/shared/protocol.js`: `≥1800` → 4K,
`≥900` → 1080, `≥600` → 720, `≥300` → 360, y por debajo de 300 no se ofrece. El ladder contiene un
candidato por tier, gana el mejor y la entrada conserva la **altura real** y el `format_id`, de modo
que `media.add` sigue resolviendo con `height<=N`.

`default_quality` compara tiers: un ladder cuyo tier 720 tiene altura real 888 responde `720`, que es
la etiqueta que la extensión preselecciona.

### Localización del menú contextual

`contextMenuAddTorrent` existe ahora en `en`, `es` y `pt_BR`. El test de locales pasó a escanear
`background.js` y los tres idiomas: antes miraba sólo los cuatro scripts de página y el catálogo
español, que es exactamente por qué la clave faltante pasó desapercibida.

## Verificación en vivo

Prueba en vivo en el equipo del usuario (host `ssh ubuntu`, paquete `0.2.4-1` instalado, daemon `0.2.4`):
el sondeo real con `media.probe` sobre `https://www.youtube.com/watch?v=FBhSudzU_ck` devuelve los
formatos con alturas `1184, 888, 592, 394, 296, 196, 128` y un ladder
`1080@1184, 720@888, 360@592, audio` con `default_quality: 720`, alineado con la extensión.
La extensión preparada en `~/.local/share/segmenta/extensions/chromium-0.2.4` incluye la clave
`contextMenuAddTorrent` en sus tres idiomas (`en`, `es`, `pt_BR`). La evidencia detallada está en
[`docs/qa/segmenta-0.2.4-live.md`](https://github.com/wilkin-barban/Segmenta/blob/main/docs/qa/segmenta-0.2.4-live.md).

## Verificación

- `cargo test --workspace` → 408 tests; `cargo fmt --all -- --check`, `cargo clippy -D warnings`.
- `node extension/tests/run.mjs` → 172 tests, incluido el test que exige cada clave usada por la
  extensión en cada idioma.
- Test de regresión del daemon con las alturas reales de `FBhSudzU_ck`: el ladder queda
  `1080/720/360/audio` con alturas `1184/888/592` y `default_quality` responde `720`.
- `packaging/check-package.sh dist/segmenta_0.2.4-1_amd64.deb 0.2.4` → `paths=23/23`.

`.deb` sha256: `c9f9f8d958c25029e17b1e822b6af4d6a5551a5b1fcdec0993e841bfb0aedc1d`.

## Instalación

```bash
sudo apt install ./segmenta_0.2.4-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

Después, en Preferencias → Navegadores: **Preparar** la extensión y **Recargar** la tarjeta de
Segmenta en `chrome://extensions` (Chromium) o `about:debugging#/runtime/this-firefox` (Firefox).

---

### Todas las releases

[0.2.4](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.4) · [0.2.3](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.3) · [0.2.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.1) · [0.2.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.0) · [0.1.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.2) · [0.1.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.1) · [0.1.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.0) · [0.0.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.2) · [0.0.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1)
