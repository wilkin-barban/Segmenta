<p align="center">
  <img src="https://raw.githubusercontent.com/wilkin-barban/Segmenta/main/packaging/io.github.wilkin.Segmenta.svg" width="96" alt="Icono de Segmenta">
</p>

# Segmenta 0.2.3

El panel de YouTube vuelve a ofrecer resoluciones de video, la extensión avisa cuando la copia
cargada no es la del paquete instalado y el menú contextual dejó de duplicarse. Esta release reúne
lo que quedó pendiente desde 0.2.1.

## Qué problema resuelve

Tres defectos de la extensión se veían juntos en YouTube:

1. **Sólo audio.** El panel ofrecía «Solo audio» en videos que sí tenían resoluciones. La causa no
   era YouTube: `media.probe --kind video` devuelve las alturas reales de cada pista
   (`1184, 888, 592, 394, 296, 196, 128` en `FBhSudzU_ck`) y el ladder `qualities` del daemon traía
   únicamente audio. `groupQualities()` conservaba sólo las alturas exactas `2160/1080/720/360`, no
   encontraba ninguna coincidencia y caía al fallback de sólo audio.
2. **Menú duplicado.** Chrome reportaba `Cannot create item with duplicate id
   segmenta-add-torrent`: el service worker creaba el menú contextual en cada arranque sin limpiar
   el anterior.
3. **Copia vieja.** El arreglo del menú no llegaba a quien tenía cargada una extensión antigua, y
   no había forma de verlo desde la aplicación.

## Cambios

### Calidades por tier nominal

Cada altura real cae en un tier —`≥1800` → 4K, `≥900` → 1080, `≥600` → 720, `≥300` → 360— y el
mejor candidato del tier conserva su **altura real** y su `format_id`. Como `media.add` resuelve con
`height<=N`, enviar 888 sigue seleccionando la pista de 888 y nunca sobrepasa el tier. La
preselección también es por tier: un grupo de 888 marca **720p**, no el más cercano por altura. Las
alturas por debajo de 360 quedan fuera de la escalera documentada.

### Detección de versión de la extensión

`extension.status` suma, por navegador, `prepared_version`, `shipped_version` y `version_sync`
(`match`, `mismatch`, `not-prepared`). Preferencias → **Navegadores** advierte cuando la copia
preparada no es la del paquete, nombra ambas versiones y su acción pasa a **Recargar**, que abre la
página de extensiones del navegador. Un par no verificable nunca se informa como coincidencia y un
daemon viejo no dispara falsas advertencias.

### Menú contextual idempotente

`setupContextMenus()` llama a `removeAll()` antes de crear `segmenta-add-torrent` y consume el error
de la promesa, así que rearrancar el service worker ya no duplica el id ni deja entradas muertas.
Cubierto por un test de doble arranque.

### Enlaces cortos y mixes

`https://youtu.be/…` y las URLs con `list=` se reconocen como un solo video y `yt-dlp` recibe
`--no-playlist`: descarga la pista pedida, no la lista completa de un mix.

## Prueba en vivo

Verificado por el usuario sobre el paquete 0.2.3 instalado en Ubuntu 26.04, recargando la extensión
`0.2.3` preparada: el panel ya ofrece opciones de descarga en el video del reporte y la descarga se
completa. La evidencia y los comandos quedaron en
[`docs/qa/segmenta-0.2.3-live.md`](https://github.com/wilkin-barban/Segmenta/blob/main/docs/qa/segmenta-0.2.3-live.md).
No hay capturas simuladas ni automatización de navegador en esa verificación.

## Verificación

- `node extension/tests/run.mjs` → 172 tests.
- `cargo test --workspace` → 407 tests; `xvfb-run -a cargo test -p segmenta-gui` → 95 tests.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- `node extension/tools/sync-shared.mjs --check`, `node extension/tools/pack.mjs --check`.
- `packaging/check-package.sh dist/segmenta_0.2.3-1_amd64.deb 0.2.3` → `paths=23/23`.

`.deb` sha256: `4c9726c88ab160671d9dffb61e8a3ffb3b83955546f9ef67f7404b117359d4a0`.

## Instalación

```bash
sha256sum -c segmenta_0.2.3-SHA256SUMS
sudo apt install ./segmenta_0.2.3-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

Después, en Preferencias → Navegadores: **Preparar** la extensión y **Recargar** la tarjeta de
Segmenta en `chrome://extensions` (Chromium) o `about:debugging#/runtime/this-firefox` (Firefox).

---

### Todas las releases

[0.2.3](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.3) · [0.2.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.1) · [0.2.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.0) · [0.1.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.2) · [0.1.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.1) · [0.1.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.0) · [0.0.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.2) · [0.0.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1)
