<p align="center">
  <img src="https://raw.githubusercontent.com/wilkin-barban/Segmenta/main/packaging/io.github.wilkin.Segmenta.svg" width="96" alt="Icono de Segmenta">
</p>

# Segmenta 0.2.3

La extensión vuelve a ofrecer resoluciones de video cuando YouTube reporta alturas que no son las
de la escalera nominal.

> Esta versión **no** está publicada como tag ni Release de GitHub: es el paquete local de
> remediación. El `.deb` y los ZIP se generan desde el árbol con los comandos de abajo.

## Qué problema resuelve

En 0.2.2 el panel de YouTube sólo ofrecía **sólo audio** en videos que sí tenían resoluciones. La
causa no era YouTube: `media.probe --kind video` devuelve las alturas reales de cada pista
(`1184, 888, 592, 394, 296, 196, 128` en `FBhSudzU_ck`) y el ladder `qualities` del daemon traía
únicamente audio. `groupQualities()` conservaba sólo las alturas exactas `2160/1080/720/360`, no
encontraba ninguna coincidencia, y el panel caía al fallback de sólo audio.

## Cambios

- **Agrupación por tier nominal**: cada altura real cae en un tier —`>=1800` → 4K, `>=900` → 1080,
  `>=600` → 720, `>=300` → 360— y el primer candidato del tier (el mejor ordenado por el daemon)
  conserva su **altura real** y su `format_id`. Como `media.add` resuelve con `height<=N`, enviar
  888 sigue seleccionando la pista de 888 y nunca sobrepasa el tier.
- **Preselección por tier**: `defaultQuality()` compara tiers, así que un grupo de 888
  preselecciona **720** en vez de caer al más cercano por altura.
- Las alturas por debajo del tier más bajo (240, 144) siguen fuera de la escalera documentada, para
  que el panel nunca renderice la lista cruda del backend.

## Prueba del caso real

Con el probe de `FBhSudzU_ck` (formatos `1184/888/592/394/296/196/128`, `qualities` sólo audio) el
panel ahora ofrece **1080p, 720p, 360p y Sólo audio**, con 720 preseleccionado. Cubierto por dos
tests en `extension/tests/quality.test.mjs`, incluido el caso `1920×888` / `1280×592`.

## Instalación local

```bash
node extension/tools/pack.mjs
packaging/fetch-ytdlp.sh
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.2.3-1_amd64.deb 0.2.3
sudo apt install ./dist/segmenta_0.2.3-1_amd64.deb
```

Después, en Preferencias → Navegadores: **Preparar** la extensión y **Recargar** la tarjeta de
Segmenta en `chrome://extensions` (Chromium) o `about:debugging#/runtime/this-firefox` (Firefox).

## Verificación

- `node extension/tests/run.mjs` (172 tests), `node extension/tools/sync-shared.mjs --check`,
  `node extension/tools/pack.mjs --check`.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `xvfb-run -a cargo test -p segmenta-gui`.
- `packaging/check-package.sh dist/segmenta_0.2.3-1_amd64.deb 0.2.3`.

---

### Todas las releases

[0.2.2](https://github.com/wilkin-barban/Segmenta/releases) · [0.2.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.1) · [0.2.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.0) · [0.1.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.2) · [0.1.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.1) · [0.1.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.0) · [0.0.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.2) · [0.0.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1)
