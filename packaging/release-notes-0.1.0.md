# Segmenta 0.1.0

Primera release con descarga de medios y extensión de navegador. Mantiene el motor de
descargas directas de 0.0.2 y agrega YouTube, HLS/DASH públicos y un panel flotante para
elegir calidad, audio-only y playlists.

## Qué cambia

### Extensión de navegador

- Panel flotante en YouTube con título, miniatura, calidades de video, **Solo audio** y
  límite de playlist (1–500).
- Targets separados:
  - Chromium MV3 para Chrome, Chromium, Edge, Brave, Vivaldi y Opera.
  - WebExtension MV3 para Firefox (`segmenta@wilkin.github.io`).
- Native Messaging con host `/usr/bin/segmenta-host`; el `.deb` instala los siete
  manifiestos nativos.
- Mensajes validados (origen, tipo, URL y tamaño), allowlist de YouTube, sin métodos
  arbitrarios al host y errores traducidos en el panel.
- Paquetes reproducibles con `_locales` (es/en/pt-BR) incluidos y validados por
  `extension/tools/pack.mjs`.

### Medios

- `segmenta media probe` y `segmenta media add` en la CLI y en el panel.
- YouTube: video, audio-only y playlists con límite.
- Selector de video con fallback muxed/DASH:
  `b[height<=H]/18/bv*[height<=H]+ba/b[height<=H]/best`.
- HLS y DASH públicos con `SegmentTemplate`: descarga de segmentos, progreso real y
  remux con `ffmpeg -c copy`.
- Nombres únicos por tarea (`… (1).m4a`) y `--no-continue` para no reanudar archivos
  ajenos.
- Progreso persistido (bytes, total, velocidad, ETA) y tamaño final verificado a partir
  del archivo o directorio de playlist.
- Validación de versión mínima de `yt-dlp` antes de tocar la red.

### Correcciones de QA

- Callback + Promise de `chrome.runtime.sendNativeMessage`.
- Locales de la extensión.
- CLI/GUI de medios visible en `--help`.
- Fallback de formatos de YouTube y errores 403/416.
- Probe de playlist acotado con `--playlist-end` para no fallar por entradas ocultas.
- DASH `SegmentList`/`SegmentBase` queda explícitamente fuera de alcance (`1043`);
  0.1.0 soporta `SegmentTemplate`.

## Requisitos

- Ubuntu 26.04 (paquete `.deb`).
- **`yt-dlp >= 2026.10.01`** en `PATH`. El 2026.03 de Ubuntu devuelve 403/formatos
  obsoletos; el daemon falla con un mensaje accionable si la versión es vieja.
- `ffmpeg` para HLS/DASH y el merge de video+audio.
- Chrome/Chromium/Edge/Brave/Vivaldi/Opera o Firefox. El add-on de Firefox se puede
  cargar temporalmente; el snap de Firefox no permite la conexión remota de `web-ext`,
  pero el add-on funciona en Firefox oficial.

## Instalación

```bash
sudo apt install ./segmenta_0.1.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta --version
segmenta media --help
```

Chrome/Chromium y derivados: cargar `segmenta-extension-0.1.0.zip` descomprimido con
`chrome://extensions` → **Cargar descomprimida**. Firefox: `about:debugging` →
**Cargar complemento temporal** → `manifest.json` del ZIP de Firefox.

## Verificación

- `cargo test --workspace`: 45 suites.
- GUI bajo Xvfb: OK.
- Extensión: 67 tests Node.
- `cargo clippy --workspace --all-targets -- -D warnings`: 0.
- `cargo fmt --check`: OK.
- `packaging/check-package.sh`: 18/18.
- QA en vivo en Ubuntu: video 720p, audio, repetición sin 416, playlists de 2 videos,
  HLS público, Chrome for Testing y Firefox oficial.

## Artefactos

- `segmenta_0.1.0-1_amd64.deb` — 4.167.904 B —
  `a8c62a4a66623357468329fd16a0c55c98a08e016f88ba92a7c0ad2006c7bd99`
- `segmenta-extension-0.1.0.zip` — 35.413 B —
  `137f95b38769d32235dece579366ab14b1241aa20302ae919ab717b8b6d62626`
- `segmenta-firefox-0.1.0.zip` — 35.127 B —
  `8242c92381a459e08353bcef79bd0682ba25d41d9087a7e7770c6c7b355270d9`

## Limitaciones conocidas

- DASH con `SegmentList`/`SegmentBase` no está soportado en 0.1.0.
- DRM/AES y streams en vivo se rechazan explícitamente.
- Sin subtítulos ni multi-audio.
- El `.deb` es el único formato de instalación.
- El snap de Firefox no permite automatizar `web-ext`; usar Firefox oficial para QA
  automatizado.
