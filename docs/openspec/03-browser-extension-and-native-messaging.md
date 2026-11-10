# OpenSpec 03: Extensión de Navegador y Host de Mensajería Nativa

> **Especificación:** OPENSPEC-SEGMENTA-003  
> **Versión:** 1.0.0  
> **Área:** Integración Web, Sniffer Multimedia y Native Messaging

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
  * Emite un evento `window.postMessage` con los metadatos de la solicitud:
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
Debe colocarse en las rutas estándar de Linux según el navegador:

* **Google Chrome / Chromium**:  
  `~/.config/google-chrome/NativeMessagingHosts/io.github.wilkin.segmenta.json`
* **Brave Browser**:  
  `~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/io.github.wilkin.segmenta.json`
* **Mozilla Firefox**:  
  `~/.mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json`

Contenido del archivo JSON:
```json
{
  "name": "io.github.wilkin.segmenta",
  "description": "Host de mensajería nativa para Segmenta en Ubuntu",
  "path": "/usr/bin/segmenta-host",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://[EXTENSION_ID]/"
  ],
  "allowed_extensions": [
    "segmenta@wilkin.github.io"
  ]
}
```

---

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
3. **Muxing y Remuestreo sin Pérdida**:
   * Al completarse los fragmentos, el daemon invoca de forma transparente a `ffmpeg` en Linux:
     ```bash
     ffmpeg -i video_stream.m4s -i audio_stream.m4s -c copy -movflags +faststart output.mp4
     ```
   * El usuario obtiene un archivo `.mp4` perfectamente sincronizado y con metadatos completos.
