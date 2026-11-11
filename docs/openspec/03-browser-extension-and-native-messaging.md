# OpenSpec 03: Extensión de Navegador y Host de Mensajería Nativa

> **Especificación:** OPENSPEC-SEGMENTA-003  
> **Versión:** 1.0.0  
> **Área:** Integración Web, Sniffer Multimedia y Native Messaging

> **Normativo:** los manifiestos separados de Chrome/Firefox, la clave de extensión, la inyección
> en `world: MAIN`, el framing little-endian y el alcance de streaming están en
> [`09-extension-security.md`](09-extension-security.md). El contrato HTTP está en 06 y el IPC en 07.

---

## 1. Visión General de la Integración

La integración con navegadores permite a Segmenta capturar descargas de forma transparente cuando el usuario hace clic en un enlace o cuando se reproduce un video en streaming (YouTube, Vimeo, Twitter, etc.).

```
[ Página Web (DOM / Fetch / XHR / Media) ]
                 |
        (Monkey-Patch Hook)
                 v
           [ document.js ]
                 |
          window.postMessage
                 v
           [ content.js ] (Botón flotante "Descargar con Segmenta")
                 |
        chrome.runtime.sendMessage
                 v
         [ background.js ] (Service Worker MV3)
                 |
        chrome.runtime.connectNative("io.github.wilkin.segmenta")
                 v
         [ segmenta-host ] (Binario ELF en Linux)
                 |
        Unix Domain Socket
                 v
        [ segmenta-daemon ]
```

---

## 2. Especificación de la Extensión (Manifest V3)

### 2.1. Archivo `manifest.json`
El siguiente `manifest.json` es **ilustrativo**: la separación de contextos, los manifiestos de
Chrome/Firefox y la inyección válida están en 09. En particular, `document.js` va en
`content_scripts` con `world: "MAIN"` (09 §3.1), no hace falta exponerlo en `web_accessible_resources`,
y `allowed_origins` no se mezcla con `allowed_extensions`.

```json
{
  "manifest_version": 3,
  "name": "Segmenta Integration Module",
  "version": "1.0.0",
  "description": "Módulo de captura profunda e intercepción multimedia para Segmenta en Ubuntu",
  "default_locale": "es",
  "icons": {
    "16": "icons/logo-16.png",
    "48": "icons/logo-48.png",
    "128": "icons/logo-128.png"
  },
  "action": {
    "default_icon": {
      "16": "icons/logo-16.png",
      "32": "icons/logo-32.png"
    },
    "default_title": "Segmenta Download Manager"
  },
  "background": {
    "service_worker": "background.js"
  },
  "content_scripts": [
    {
      "matches": ["<all_urls>"],
      "js": ["content.js"],
      "run_at": "document_start",
      "all_frames": true
    }
  ],
  "web_accessible_resources": [
    {
      "resources": ["document.js", "panel.html", "panel.js", "panel.css"],
      "matches": ["<all_urls>"]
    }
  ],
  "permissions": [
    "nativeMessaging",
    "downloads",
    "cookies",
    "webRequest",
    "declarativeNetRequest",
    "scripting",
    "contextMenus",
    "storage"
  ],
  "host_permissions": [
    "<all_urls>"
  ]
}
```

---

## 3. Mecanismo de Sniffing e Inyección en Página

### 3.1. Hook de Red en la Página (`document.js`)
Para no perder peticiones AJAX o llamadas a API de video creadas dinámicamente por librerías (como Shaka Player, Hls.js o Dash.js):
* `document.js` intercepta `window.fetch` y `XMLHttpRequest.prototype.open`.
* Si la URL solicitada coincide con extensiones multimedia (`.m3u8`, `.mpd`, `.mp4`, `.m4s`, `.webm`, `.ts`):
  * Emite los metadatos por el canal seguro definido en 09 (MessageChannel/canal validado); no se confía en un `postMessage` con origen `*`. El siguiente bloque es sólo ilustrativo:
    ```javascript
    window.postMessage({
      source: "segmenta-sniffer",
      type: "media_detected",
      url: requestUrl,
      method: "GET",
      mimeType: detectedMimeType
    }, "*");
    ```

### 3.2. Panel Flotante de Descarga (`content.js`)
* Observa los elementos `<video>` y `<audio>` del DOM mediante un `MutationObserver`.
* Cuando un elemento multimedia comienza a reproducirse o se detecta un manifiesto de streaming:
  * Inyecta un botón flotante superpuesto en la esquina superior derecha del reproductor:
    `[ ⬇ Descargar este video ]`
  * Al hacer clic, muestra las calidades detectadas (1080p, 720p, 480p, solo audio) y envía la orden a `background.js`.

---

## 4. El Host de Mensajería Nativa (`segmenta-host`)

### 4.1. Archivo de Manifiesto en Ubuntu
Chrome/Brave y Firefox usan manifiestos **separados**; no se debe usar un único JSON con
`allowed_origins` y `allowed_extensions` a la vez. Los manifiestos completos, la clave pública fija y
el ID determinista están en [`09-extension-security.md`](09-extension-security.md) §1–§2. El host se
instala con el paquete de 10 y, en desarrollo, se usa un manifiesto separado con el ID de la extensión
sin empaquetar.

### 4.2. Protocolo de E/S por `stdio`
El binario ELF `segmenta-host` implementa el protocolo de Chromium:

1. **Lectura de Solicitud**:
   * Lee los primeros 4 bytes de `stdin` como un entero de 32 bits en little-endian:
     $$\text{longitud} = \text{uint32\_from\_bytes}(b_0, b_1, b_2, b_3)$$
   * Lee exactamente $\text{longitud}$ bytes de `stdin` como cadena codificada en UTF-8 (JSON).
2. **Reenvío al Socket Unix**:
   * Abre conexión con el socket local: `$XDG_RUNTIME_DIR/segmenta/daemon.sock`.
   * Si el socket no responde (el daemon no está activo), lanza el servicio en segundo plano mediante `systemctl --user start segmenta.service` o ejecuta el binario en segundo plano.
   * Envía el mensaje JSON al daemon y espera respuesta.
3. **Escritura de Respuesta**:
   * Escribe los 4 bytes de longitud de la respuesta JSON en `stdout`.
   * Escribe la respuesta JSON en `stdout` y ejecuta `fflush(stdout)`.

---

## 5. Procesamiento de Flujos HLS y MPEG-DASH

Cuando el recurso capturado es un manifiesto (`.m3u8` o `.mpd`):
1. La extensión no descarga el archivo directamente; envía la URL del manifiesto al daemon con el flag `"is_stream": true`.
2. `segmenta-daemon` ejecuta el parser interno de streams:
   * **HLS**: Lee la lista maestra M3U8, extrae la lista de fragmentos `.ts` o `.m4s`, y descarga cada segmento concurrentemente.
   * **DASH**: Lee el XML `.mpd`, calcula la línea de tiempo (`SegmentTimeline`), y descarga las pistas de video y audio independientes.
3. **Muxing sin Pérdida**:
   * El pipeline completo —init segment, orden de fragmentos, listas `ffconcat`, audio/video por
     separado y ejecución sin shell— está especificado en
     [`09-extension-security.md`](09-extension-security.md) §5.2. El comando no debe usar un shell.
   * **Alcance v1:** HLS/DASH públicos sin cifrar. Cualquier `EXT-X-KEY` distinto de `NONE` o
     `ContentProtection` DRM se rechaza con un mensaje explícito.

---

## 14. Media, navegadores y alcance de 0.1.0

Esta sección fija el contrato de la release 0.1.0. No reemplaza lo anterior: lo extiende.

### 14.1. Matriz de navegadores

La extensión se publica para la **familia Chromium completa y Firefox**. Misma UI y mismo
`background`; sólo cambia el manifiesto y el adaptador de API.

| Navegador | Extensión | Manifiesto nativo de usuario |
| --- | --- | --- |
| Chrome | MV3 con `key` fija | `${XDG_CONFIG_HOME}/google-chrome/NativeMessagingHosts/io.github.wilkin.segmenta.json` |
| Chromium | MV3 | `${XDG_CONFIG_HOME}/chromium/NativeMessagingHosts/...` |
| Brave | MV3 | `${XDG_CONFIG_HOME}/BraveSoftware/Brave-Browser/NativeMessagingHosts/...` |
| Edge | MV3 | `${XDG_CONFIG_HOME}/microsoft-edge/NativeMessagingHosts/...` |
| Vivaldi | MV3 | `${XDG_CONFIG_HOME}/vivaldi/NativeMessagingHosts/...` |
| Opera | MV3 | `${XDG_CONFIG_HOME}/opera/NativeMessagingHosts/...` |
| Firefox | WebExtension con `browser_specific_settings.gecko.id = segmenta@wilkin.github.io` | `${XDG_CONFIG_HOME}/mozilla/native-messaging-hosts/segmenta@wilkin.github.io.json` |

- El nombre del host es siempre `io.github.wilkin.segmenta`; el binario es `/usr/bin/segmenta-host`.
- Chromium identifica la extensión por su `key`; el ID derivado debe aparecer como
  `allowed_origins: ["chrome-extension://<ID>/"]`. Firefox usa `allowed_extensions`.
- El host es común: recibe del navegador su propio manifiesto y no necesita lógica por navegador.
- `extension/install-native.sh` detecta los navegadores instalados y escribe los manifiestos de
  usuario; el `.deb` instala los manifiestos de sistema de los soportados.
- Firefox sin firma de tienda se carga temporalmente desde `about:debugging`; es una limitación
  documentada, no un fallo.

### 14.2. Flujo de YouTube

1. `content.js` detecta `youtube.com/watch` o `youtube.com/playlist` e inyecta el botón flotante
   «Descargar con Segmenta» en un Shadow DOM.
2. Sólo tras un click pide `media.probe` al `background`, que reenvía por Native Messaging.
3. El panel muestra las calidades devueltas por el daemon, «Solo audio», «Video actual» y
   «Playlist completa (N)» con límite editable (default 100, máximo 500).
4. Al confirmar se envía `media.add` y la extensión hace polling de `media.status` cada 1,5 s.
5. Al completar, toast con la ruta y «Abrir carpeta»; al fallar, el mensaje proviene del `code`
   estable traducido por la extensión.
6. No se leen cookies, no se descarga contenido privado, no se intenta saltar DRM ni anuncios.

### 14.3. Formatos y modos

- **Video**: `yt-dlp` con el `format_id` elegido; si se pide una altura se elige el mejor formato
  progresivo o `video+audio` que la contenga (`bestvideo[height<=H]+bestaudio/best[height<=H]`).
  Contenedor final `mp4` (merge con ffmpeg, `-c copy`).
- **Audio**: `bestaudio/best`, contenedor original (M4A/Opus), **sin recodificar** por defecto.
- **Playlist**: una tarea agregada con `playlist_count`, `batch_id` y progreso
  `{completed,total}`; límite `--playlist-end`.
- **HLS/DASH**: variante/representación elegida por altura, init segment primero, fragmentos
  descargados con el scheduler, union con `ffmpeg -c copy` y argumentos separados.

### 14.4. Errores de medios

| Código | Nombre | Causa |
| --- | --- | --- |
| `1040` | `ERR_DRM_PROTECTED` | `EXT-X-KEY` distinto de `NONE` o `ContentProtection` de DRM |
| `1041` | `ERR_LIVE_NOT_SUPPORTED` | playlist sin `ENDLIST` o MPD `type="dynamic"` |
| `1042` | `ERR_MANIFEST_MALFORMED` | manifiesto ilegible o sin segmentos |
| `1043` | `ERR_STREAM_UNSUPPORTED` | subtítulos, multi-audio o formato fuera de alcance |
| `1044` | `ERR_MEDIA_BACKEND` | `yt-dlp`/`ffmpeg` ausente, falló o quedó desactualizado |
| `1045` | `ERR_MEDIA_NOT_PUBLIC` | contenido privado, con edad o no disponible |
