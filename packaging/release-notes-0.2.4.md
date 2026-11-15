<p align="center">
  <img src="https://raw.githubusercontent.com/wilkin-barban/Segmenta/main/packaging/io.github.wilkin.Segmenta.svg" width="96" alt="Icono de Segmenta">
</p>

# Segmenta 0.2.4

Calidades por tier nominal en el daemon y en la extensión, descargas que se recuperan solas de un
fallo transitorio, reintento real desde la interfaz y una extensión que informa sin robar el foco.

## Qué problema resolvía

Cuatro defectos se veían juntos al descargar desde YouTube:

1. **Calidades vacías o sólo audio.** El daemon agrupaba el ladder `qualities` por altura exacta
   (`2160/1080/720/360`), pero YouTube reporta alturas reales
   (`1184, 888, 592, 394, 296, 196, 128`). Para esos videos el ladder salía con **sólo audio** y
   cualquier consumidor del contrato —la CLI incluida— veía una escalera vacía.
2. **Error de localización en Chrome.** El menú contextual de torrents usaba
   `API.getMessage('contextMenuAddTorrent')` y esa clave no existía en ningún catálogo. Chrome
   registraba un `lastError` sin leer («Could not load the message») señalado en
   `shared/browser-api.js:176`, la línea de `i18n.getMessage`.
3. **Descargas marcadas como fallidas sin motivo.** Un `HTTP 403` intermitente del host de medios se
   reportaba como «el video no es público», y un `No such file or directory` de un temporal se
   reportaba como «yt-dlp no está instalado». Dos mensajes que mandaban a investigar lo que no era.
4. **Sin salida después de un fallo.** La GUI reintenta con `download.resume`, y el daemon respondía
   `1003 TASK_NOT_RESUMABLE` a cualquier tarea `Failed`: la fila quedaba muerta para siempre.

## Cambios

### Calidades por tier nominal (daemon y extensión)

`media::quality_tier` aplica los mismos cortes que `extension/shared/protocol.js`: `≥1800` → **4K**,
`≥900` → **1080p**, `≥600` → **720p**, `≥300` → **360p**, y por debajo de 300 no se ofrece. El ladder
contiene un candidato por tier, gana el mejor y la entrada conserva la **altura real** y el
`format_id`, de modo que `media.add` sigue resolviendo con `height<=N`.

`default_quality` compara tiers: un ladder cuyo tier 720 tiene altura real 888 responde `720`, que es
la etiqueta que la extensión preselecciona. El panel de la extensión usa la misma regla, así que
ambos extremos coinciden y la lista muestra 1080p / 720p / 360p / Sólo audio.

### Localización del menú contextual

`contextMenuAddTorrent` existe ahora en `en`, `es` y `pt_BR`. El test de locales pasó a escanear
`background.js` y los tres idiomas: antes miraba sólo cuatro scripts de página y el catálogo español,
que es exactamente por qué la clave faltante pasó desapercibida.

### Fallback progresivo ante un 403 transitorio

`run_download` hace **un** intento más con el formato progresivo `18` cuando el stderr es un fallo
transitorio de stream de YouTube (`403`/`Forbidden`, `unable to download video data`, bot-check,
timeout, connection reset). Cualquier otro fallo, y el segundo intento, conservan su propio código:
nada se enmascara. Un `403` ahora dice que el host rechazó ese stream y que la descarga se puede
reintentar, en lugar de «no es público».

### Reintento desde la interfaz

`download.resume` acepta tareas en `Failed`: limpia `error_code` y `error_message`, y si es una tarea
multimedia la relanza con el plan guardado en su registro (URL, tipo, altura real o `format_id` de
audio, destino). El resto vuelve a la cola como una tarea pausada. `Completed` y `Cancelled` siguen
rechazadas.

### Extensión no intrusiva

`announceAddedTask` ya no abre la ventana de Segmenta: el panel ya sigue el progreso en tiempo real,
así que la apertura automática sólo interrumpía la navegación. Se mantiene la **notificación de
sistema** con la carpeta de destino, y la ventana se abre únicamente al pulsar **Abrir Segmenta** o
**Ver progreso** (`segmenta.open` → `system.open`, enfocada en la tarea).

## Verificación en vivo

Prueba en el equipo del usuario (host Ubuntu, paquete `0.2.4-1`, daemon `0.2.4`):

- `media.probe` real de `FBhSudzU_ck`: formatos `1184, 888, 592, 394, 296, 196, 128` y ladder
  `1080@1184, 720@888, 360@592, audio` con `default_quality: 720`.
- Tres descargas consecutivas del video que antes fallaba, todas en `completed`.
- Reintento desde la interfaz sobre una tarea `Failed`: `download.resume` la relanza y termina en
  `completed` con 26 841 221 bytes en la ruta original.
- La extensión preparada incluye `contextMenuAddTorrent` en los tres idiomas y ya no contiene la
  apertura automática de la ventana.

Evidencia detallada en [`docs/qa/segmenta-0.2.4-live.md`](https://github.com/wilkin-barban/Segmenta/blob/main/docs/qa/segmenta-0.2.4-live.md).

## Verificación

- `cargo test --workspace` → 413 tests; `cargo fmt --all -- --check`, `cargo clippy -D warnings`.
- `node extension/tests/run.mjs` → 176 tests, incluido el que exige cada clave usada por la extensión
  en cada idioma y los que fijan que un add sólo notifica.
- `packaging/check-package.sh dist/segmenta_0.2.4-1_amd64.deb 0.2.4` → `paths=23/23`.

## Instalación

```bash
sha256sum -c segmenta_0.2.4-SHA256SUMS
sudo apt install ./segmenta_0.2.4-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

Después, en Preferencias → Navegadores: **Preparar** la extensión y **Recargar** la tarjeta de
Segmenta en `chrome://extensions` (Chromium) o `about:debugging#/runtime/this-firefox` (Firefox). Si el
paquete se instala sobre una versión anterior con el mismo número, `apt` no reemplaza los archivos:
usá `sudo apt install --reinstall ./segmenta_0.2.4-1_amd64.deb`.

---

### Todas las releases

[0.2.4](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.4) · [0.2.3](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.3) · [0.2.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.1) · [0.2.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.2.0) · [0.1.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.2) · [0.1.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.1) · [0.1.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.1.0) · [0.0.2](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.2) · [0.0.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.0.1)
