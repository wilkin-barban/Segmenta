<p align="center">
  <img src="https://raw.githubusercontent.com/wilkin-barban/Segmenta/main/packaging/io.github.wilkin.Segmenta.svg" width="96" alt="Icono de Segmenta">
</p>

# Segmenta 0.2.2

La extensión deja de quedarse vieja sin avisar: enlaces cortos y mixes de YouTube, un menú
contextual que ya no duplica su id y una advertencia en Preferencias cuando la copia cargada en el
navegador no es la que trae el paquete instalado.

> Esta versión **no** está publicada como tag ni Release de GitHub: es el paquete local de
> remediación. El `.deb` y los ZIP se generan desde el árbol con los comandos de abajo.

## Qué problema resuelve

Chrome mostraba `Cannot create item with duplicate id segmenta-add-torrent` sobre YouTube aunque el
backend sí detectaba el video (`formats=7`). Había dos causas encadenadas: el service worker volvía
a crear el menú contextual en cada arranque sin limpiar el anterior, y la extensión cargada en el
navegador seguía siendo una copia vieja sin los arreglos. La primera está corregida en el código; la
segunda ahora es visible.

## Cambios

### Enlaces cortos y mixes

`https://youtu.be/FBhSudzU_ck?list=RD-…` y las URLs con `list=` se reconocen como un solo video: el
panel se abre y `yt-dlp` recibe `--no-playlist`, así que descarga la pista pedida y no la lista
completa de un mix.

### Menú contextual idempotente

`setupContextMenus()` llama a `removeAll()` antes de crear `segmenta-add-torrent` y consume el error
de la promesa. Rearrancar el service worker (o recargar la extensión) ya no duplica el id ni deja
entradas muertas en el menú. Cubierto por un test de doble arranque.

### Detección de versión

`extension.status` suma tres campos por navegador:

| Campo | Valor |
| --- | --- |
| `prepared_version` | `version` del `manifest.json` de la copia preparada, o `null` |
| `shipped_version` | `version` del ZIP que trae el paquete, con la versión del paquete como respaldo |
| `version_sync` | `match`, `mismatch` o `not-prepared` |

Preferencias → **Navegadores** advierte cuando `version_sync` es `mismatch`: la fila dice qué versión
está cargada y cuál trae el paquete, y su acción pasa a **Recargar**, que abre la página de
extensiones del navegador. Un par que no se puede comparar nunca se informa como coincidencia, y un
daemon viejo (sin los campos) no dispara falsas advertencias.

## Instalación local

```bash
node extension/tools/pack.mjs
packaging/fetch-ytdlp.sh
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.2.2-1_amd64.deb 0.2.2
sudo apt install ./dist/segmenta_0.2.2-1_amd64.deb
```

Después, en Preferencias → Navegadores: **Preparar** la extensión y **Recargar** la tarjeta de
Segmenta en `chrome://extensions` (Chromium) o `about:debugging#/runtime/this-firefox` (Firefox).

## Verificación

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace` y `xvfb-run -a cargo test -p segmenta-gui`.
- `node extension/tests/run.mjs`.
- `packaging/check-package.sh dist/segmenta_0.2.2-1_amd64.deb 0.2.2`.

---

### Todas las releases

[0.2.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.1) · [0.2.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.0) · [0.1.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.2) · [0.1.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.1) · [0.1.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.0) · [0.0.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.2) · [0.0.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1)
