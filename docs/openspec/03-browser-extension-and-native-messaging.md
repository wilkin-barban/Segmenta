# OpenSpec 03: Extensión de Navegador y Host de Mensajería Nativa

> **Especificación:** OPENSPEC-SEGMENTA-003  
> **Versión:** 1.2.0  
> **Área:** Integración Web, Sniffer de YouTube y Native Messaging

> **Normativo:** los manifiestos separados de Chrome/Firefox, la clave de extensión, la inyección
> en `world: MAIN`, el framing little-endian y el alcance de streaming están en
> [`09-extension-security.md`](09-extension-security.md). El contrato HTTP está en 06 y el IPC en 07.

> **Alcance de esta revisión:** la extensión publicada es **0.1.2**
> (`extension/*/manifest.json`) y 0.2.0 está **en preparación**. Lo que aquí se describe sin
> etiqueta existe hoy en `extension/` y `crates/` en `main`; la reconciliación de cold-start (§5.4)
> ya está en `main` pero pertenece a 0.2.0 y aún no se ha publicado. Lo que 0.2.0 promete y no
> existe todavía se marca con **«(0.2, en preparación)»**.
>
> **Corrección de alcance:** el sniffer es **sólo YouTube**. No hay adaptadores de Vimeo, Twitter/X,
> Netflix, Twitch ni de ningún otro sitio, y este documento ya no los promete. La captura de
> archivos genéricos es un camino **distinto** (§5) que no usa el sniffer.

---

## 1. Visión General de la Integración

La integración con navegadores cubre **dos caminos independientes**:

1. **Sniffer de YouTube** (sólo `https://www.youtube.com/*`): detecta el video o la playlist que el
   usuario está viendo y ofrece calidades agrupadas mediante el panel flotante.
2. **Captura genérica de descargas** (`chrome.downloads`): observa las descargas que inicia el
   navegador y, si la extensión o el `.torrent`/`magnet` están permitidos en los ajustes, ofrece
   pasar el archivo a Segmenta con una ventana de confirmación.

Ambos caminos terminan en el mismo host de Native Messaging, pero **no comparten detección**: el
sniffer nunca se activa fuera de YouTube y la captura genérica **no pide `media.probe` ni
`media.add`**. La única llamada `media.*` que hace el camino de captura es `media.status`, para
comprobar que el daemon ya domina la tarea antes de borrar la copia del navegador (§5.2).

```
[ Página de YouTube (DOM / fetch / XHR) ]            [ Descarga del navegador ]
                 |                                                  |
        document.js (MAIN world)                        chrome.downloads.onCreated
                 | MessagePort privado                              |
                 v                                                  v
          content.js (ISOLATED)  ──panel flotante──▶  background.js (service worker MV3)
                 |                                                  |
        chrome.runtime.sendMessage                    alarms watchdog + confirm.html
                 |                                                  |
                 └───────────────┬──────────────────────────────────┘
                                 v
              chrome.runtime.connectNative("io.github.wilkin.segmenta")
                                 v
                  [ segmenta-host ] (binario ELF en Linux)
                                 v
                  [ segmenta-daemon ] (Unix Domain Socket)
```

---

## 2. Manifiestos reales (MV3, 0.1.2)

`extension/chromium/manifest.json` y `extension/firefox/manifest.json` son los archivos normativos;
09 detalla la separación de contextos. Resumen verificable:

| Campo | Chromium | Firefox |
| --- | --- | --- |
| Identidad | `key` RSA fija ⇒ ID `kdeldnifdbpemnhfpmhonlnibbhnoaan` | `browser_specific_settings.gecko.id = segmenta@wilkin.github.io` |
| Fondo | `background.service_worker: background.js` | `background.scripts`: `shared/browser-api.js`, `shared/protocol.js`, `shared/validate.js`, `background.js` |
| `permissions` | `nativeMessaging`, `storage`, `scripting`, `activeTab`, `downloads`, `contextMenus`, `notifications`, `alarms` | idénticos |
| `host_permissions` | `https://www.youtube.com/*` | `https://www.youtube.com/*` |
| `optional_host_permissions` | `https://*/*` | `https://*/*` |
| `content_scripts` | `document.js` (`world: MAIN`) y `shared/browser-api.js`, `shared/protocol.js`, `shared/panel.js`, `content.js` (`ISOLATED`), ambos con `matches: https://www.youtube.com/*` y `run_at: document_start` | idénticos |
| UI | `action.default_popup: popup.html`, iconos 16/32/48/128 | idénticos |

Notas que este documento fija para no contradecir el código:

* **No se pide `cookies`**, ni `webRequest`, ni `declarativeNetRequest`. El manifiesto ilustrativo de
  versiones anteriores de esta especificación (con `cookies` y `<all_urls>`) ya no existe.
* `document.js` **no** se declara en `web_accessible_resources`; va como `content_script` en el mundo
  MAIN.
* `default_locale: es`; todos los textos visibles salen de `_locales` (`es`, `en`, `pt_BR`).
* El `minimum_chrome_version` es `111` y Firefox exige `strict_min_version` `128.0`.

---

## 3. Sniffer de YouTube

### 3.1. Hook de red en la página (`document.js`)
* Intercepta `window.fetch` y `XMLHttpRequest` y **sólo** reporta URLs `https:` que terminen en
  `.m3u8` o `.mpd` (`MANIFEST_PATTERN`); no inspecciona otros recursos ni acepta otros esquemas.
* Entrega la URL por el `MessagePort` que recibe tras el handshake `segmenta:sniffer:v1`, después de
  comprobar `event.source === window`. Nunca usa `window.postMessage(..., "*")` para los datos.
* El script está marcado con `__segmentaSnifferInstalled` para no instalarse dos veces.

### 3.2. Panel flotante (`content.js` + `shared/panel.js`)
* Se inyecta únicamente en `https://www.youtube.com/*`, con `run_at: document_start`.
* Se monta cuando la ruta es `/watch` o `/playlist`; se re-sincroniza con `yt-navigate-finish` y
  `popstate`, así que sobrevive a la navegación SPA de YouTube sin recargar la página.
* La UI vive en un **Shadow DOM** (`#segmenta-extension-root`): un botón flotante en la esquina
  inferior derecha que abre el panel. No hay botón inyectado sobre el reproductor ni
  `MutationObserver` sobre elementos `<video>`.
* **Ningún `media.probe` se envía sin un click del usuario**: el botón flotante dispara el probe
  (`config.onProbe`) y el panel se pinta cuando llega la respuesta; sin click no hay llamada al
  daemon ni al host.

### 3.3. Panel de calidades de 0.1.2
* El daemon devuelve una escalera agrupada (`media::QUALITY_HEIGHTS`) y el panel renderiza **sólo**
  esa escalera, deduplicada por altura y en este orden:
  * `4K` (2160p), `1080p`, `720p`, `360p`, y una única entrada **«Solo audio»**.
* Las alturas fuera de la escalera (480p, 240p, …) se descartan: no se muestra la lista cruda de
  formatos de `yt-dlp`.
* **Default: 720p** (`DEFAULT_QUALITY_HEIGHT`); si 720p no existe, se marca la altura más cercana.
  La misma regla se aplica en `protocol.js` (`defaultQuality`) y en el daemon
  (`media::default_quality`).
* Entradas adicionales, sólo cuando aplican:
  * `Playlist completa (N)` con límite editable (default 100, máximo 500) si el probe reporta
    playlist;
  * un objetivo de stream detectado con la URL del manifiesto.
* El envío usa `media.add` con `mode` (`video`, `audio`, `playlist`) y `height` o `formatId`; el
  panel muestra el handoff con el `task_id` y sigue el progreso con `media.status` cada 1,5 s hasta
  `completed`, `failed` o `cancelled`.
* Los errores del daemon se traducen por `code` estable (`ERR_DRM_PROTECTED`, `ERR_LIVE_NOT_SUPPORTED`,
  `ERR_MANIFEST_MALFORMED`, `ERR_STREAM_UNSUPPORTED`, `ERR_MEDIA_BACKEND`, `ERR_MEDIA_NOT_PUBLIC`,
  `ERR_HOST_UNAVAILABLE`, `ERR_HOST_TIMEOUT`, …); el texto del daemon nunca se muestra crudo.

### 3.4. Formatos y modos (contrato de `media.add` en 07 §3.7)
- **Video**: `yt-dlp` con la altura elegida; contenedor final `mp4` (merge con ffmpeg, `-c copy`).
- **Audio**: `bestaudio/best`, contenedor original (M4A/Opus), **sin recodificar** por defecto.
- **Playlist**: una tarea agregada con `playlist_count`, `batch_id` y progreso
  `{completed,total}`; límite `--playlist-end`.
- **HLS/DASH públicos**: variante elegida por altura, init segment primero, fragmentos descargados
  **en orden y de uno en uno por el propio daemon** (no por el planificador de segmentos del
  engine), concatenación por `ffconcat` y union final con `ffmpeg -c copy` y argumentos separados.
  Este pipeline se alcanza por `media.add` con una URL de manifiesto (CLI/GUI); la extensión **no**
  lo usa fuera de YouTube.

### 3.5. Errores de medios

| Código | Nombre | Causa |
| --- | --- | --- |
| `1040` | `ERR_DRM_PROTECTED` | `EXT-X-KEY` distinto de `NONE` o `ContentProtection` de DRM |
| `1041` | `ERR_LIVE_NOT_SUPPORTED` | playlist sin `ENDLIST` o MPD `type="dynamic"` |
| `1042` | `ERR_MANIFEST_MALFORMED` | manifiesto ilegible o sin segmentos |
| `1043` | `ERR_STREAM_UNSUPPORTED` | subtítulos, multi-audio o formato fuera de alcance |
| `1044` | `ERR_MEDIA_BACKEND` | `yt-dlp`/`ffmpeg` ausente, falló o quedó desactualizado |
| `1045` | `ERR_MEDIA_NOT_PUBLIC` | contenido privado, con edad o no disponible |

---

## 4. El Host de Mensajería Nativa (`segmenta-host`)

### 4.1. Manifiestos en Ubuntu
El host es siempre `io.github.wilkin.segmenta` y el binario `/usr/bin/segmenta-host`.

* **Sistema (los instala el `.deb`):**
  * Chrome: `/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json`
  * Chromium: `/etc/chromium/native-messaging-hosts/io.github.wilkin.segmenta.json`
  * Edge: `/etc/opt/edge/native-messaging-hosts/io.github.wilkin.segmenta.json`
  * Brave: `/etc/brave/native-messaging-hosts/io.github.wilkin.segmenta.json`
  * Vivaldi: `/etc/vivaldi/native-messaging-hosts/io.github.wilkin.segmenta.json`
  * Opera: `/etc/opera/native-messaging-hosts/io.github.wilkin.segmenta.json`
  * Firefox: `/usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json`
* **Usuario:** `extension.install-native` detecta los navegadores instalados y escribe
  `~/.config/<browser>/NativeMessagingHosts/io.github.wilkin.segmenta.json` (familia Chromium) o
  `~/.mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json` (Firefox), con `chmod 0644` y sin
  pisar un manifiesto existente (devuelve la ruta tal cual si ya está).
* **Inconsistencia abierta del script:** `extension/install-native.sh --print` (y su escritura)
  resuelve la ruta de Firefox como `$XDG_CONFIG_HOME/mozilla/native-messaging-hosts/…`, que Firefox no
  consulta; sólo la variante Rust usa `~/.mozilla`. Se documenta como hallazgo conocido, no como
  comportamiento soportado.
* **Claves separadas:** los manifiestos Chromium usan `allowed_origins`
  (`chrome-extension://kdeldnifdbpemnhfpmhonlnibbhnoaan/`) y el de Firefox usa `allowed_extensions`
  (`segmenta@wilkin.github.io`). Nunca se mezclan en un único JSON (09 §1–§2).

### 4.2. Protocolo de E/S por `stdio`
El binario ELF `segmenta-host` implementa el protocolo de Chromium:

1. **Lectura de Solicitud**:
   * Lee los primeros 4 bytes de `stdin` como un entero de 32 bits en little-endian:
     $$\text{longitud} = \text{uint32\_from\_bytes}(b_0, b_1, b_2, b_3)$$
   * Lee exactamente $\text{longitud}$ bytes de `stdin` como cadena codificada en UTF-8 (JSON).
2. **Filtro de métodos:** sólo se reenvían métodos cuyo prefijo sea `media.`, `system.`, `download.`,
   `settings.` o `torrent.`; cualquier otro termina con `METHOD_NOT_FOUND` sin salir del proceso.
3. **Reenvío al Socket Unix**:
   * Abre conexión con el socket local: `$XDG_RUNTIME_DIR/segmenta/daemon.sock`.
   * Si el socket no responde (el daemon no está activo), activa `segmenta.socket` con
     `systemctl --user start`.
   * Envía el sobre JSON-RPC y espera la respuesta con el mismo `id` (timeout 30 s).
4. **Escritura de Respuesta**:
   * Escribe los 4 bytes de longitud de la respuesta JSON en `stdout`.
   * Escribe la respuesta y ejecuta `fflush(stdout)`.
   * Las notificaciones del daemon (sin `id`) se reenvían igual, con framing propio.

---

## 5. Captura genérica de descargas (camino independiente del sniffer)

`background.js` escucha `chrome.downloads.onCreated` y aplica esta decisión, en orden:

1. Ignora descargas sin `id`/`url`, ya tratadas (incluidas las de la propia extensión,
   `byExtensionId`) y esquemas que no son `http(s)` ni `magnet:` (`blob:`, `data:`, `file:`,
   `chrome-extension:`, `moz-extension:`).
2. Lee los ajustes cacheados del daemon; sin caché no captura nada.
3. `capture_enabled === false` ⇒ no captura.
4. Host en `domain_denylist` ⇒ no captura.
5. `.torrent` ⇒ requiere `capture_torrents`; `magnet:` ⇒ requiere `capture_magnet`.
6. Cualquier otro archivo ⇒ requiere que su extensión esté en `capture_extensions`.
7. Con `capture_confirm` activo (default del daemon; la GUI todavía no expone el interruptor) abre la
   ventana de confirmación; sin confirmación ejecuta el handoff sin ventana y avisa por notificación.

### 5.1. Ventana de confirmación (`confirm.html` + `confirm.js`)
* Muestra **nombre**, **host** y **carpeta de destino**. El dominio del enlace se muestra como host;
  un `magnet:` muestra la etiqueta correspondiente. La carpeta sólo se muestra si el daemon la
  reportó (`download_dir`); si no, queda como «pendiente» — la extensión nunca inventa una ruta.
* Botones: **Aceptar** y **Rechazar**. La ventana se cierra sola 1,5 s después de una decisión.
* Timeout de decisión: **120 s** (`CAPTURE_CONFIRM_TIMEOUT_MS`), como máximo **20** confirmaciones
  pendientes a la vez; al superar ese número se abandona la más antigua.
* La decisión se envía como mensaje validado `segmenta.capture.decide`; una descarga ya decidida se
  rechaza con `ERR_CAPTURE_DECIDED`, así que un doble click no añade la tarea dos veces.

### 5.2. Orden seguro: pausar → confirmar → añadir → borrar tras bytes
Este es el invariante central y no admite atajos:

1. **Persistir y armar el watchdog** de la captura pendiente (`armPendingCapture`). Si no se puede
   persistir o no se puede crear la alarma, **no se pausa nada** y el navegador conserva la descarga.
2. **Pausar** la descarga del navegador (`downloads.pause`). Si la pausa falla, se elimina el
   registro pendiente.
3. **Confirmar**: abrir la ventana (timeout de 5 s para crearla) o seguir sin ventana si
   `capture_confirm` está desactivado.
4. **Añadir** al daemon (`download.add` o `torrent.add` según la fuente). Si el daemon falla, la
   descarga del navegador se **reanuda**.
5. **Esperar propiedad**: se consulta `media.status` hasta 60 s (cada 1 s) y se considera que el
   daemon es dueño cuando `status == completed` o `downloading` con `downloaded_bytes > 0`. Un estado
   `failed`/`error`/`cancelled` corta la espera como fallo.
6. **Borrar la copia del navegador** (`downloads.cancel` + `downloads.erase`) **sólo** después de
   verificar la propiedad. Si la propiedad no se demuestra, se reanuda la copia del navegador, se
   cancela la tarea huérfana (sin borrar archivos) y se notifica `ERR_CAPTURE_NOT_OWNED`.
7. **Limpiar** el registro pendiente en todas las salidas (aceptar, rechazar, timeout, ventana
   cerrada, host caído) y apagar la alarma cuando la lista queda vacía.

Consecuencia documentada: **el archivo nunca se pierde**. El precio es que, si el daemon no arranca,
la descarga sigue en el navegador y el usuario ve el aviso.

### 5.3. Watchdog con `alarms` (supervivencia del service worker)
Un service worker MV3 se suspende a voluntad, así que un `setTimeout` en memoria **no** puede ser la
única garantía de una descarga pausada:

* Antes de cualquier pausa se escribe un registro en `storage` bajo
  `segmenta_pending_captures_v1` con `downloadId`, `url`, `filename`, `referrer`, `windowId` y
  `deadline`.
* Se crea la alarma periódica `segmenta-capture-watchdog` con **1 minuto** de periodo (mínimo de
  Chrome en producción) y `delayInMinutes: 1`.
* Un worker nuevo rehidrata los registros y los resuelve por el mismo barrido: `deadline` vencido,
  ventana de confirmación desaparecida, `windowId` desconocido o registro ilegible ⇒ se **reanuda**
  la descarga del navegador; un registro con ventana viva se conserva y se rearma la alarma.
* `runtime.onStartup` dispara el mismo barrido, porque un navegador recién abierto no tiene ventanas
  que explicar los registros viejos.
* `windows.onRemoved` sobre la ventana de confirmación abandona la captura y reanuda la descarga.
* El barrido toma el registro fresco dentro de un mutex (`withPendingStore`) antes de escribir, de
  modo que una captura armada durante el barrido no se pierde ni se apaga su alarma.
* Si la API `alarms` no existe, la captura **no se pausa**: `armPendingCapture` devuelve fallo.

### 5.4. Límite de cold-start (documentado, no un bug)
Si el navegador entrega `downloads.onCreated` y la descarga ya terminó antes de que exista worker, no
hay nada que pausar y la captura se pierde. Es una carrera irreducible del modelo MV3 y así se
documenta. La reconciliación de R1 **ya está en `main`** (entra en 0.2.0, aún sin publicar): al
arrancar, al instalarse y en un tick del watchdog limitado a 5 minutos, el worker pide
`downloads.search({state:'in_progress'})`, examina como máximo 20 descargas por barrido y pasa cada
una por la misma puerta que una descarga viva (ajustes, extensión, denylist, armar-antes-de-pausar y
reglas de propiedad). Reduce la ventana; no la elimina.

---

## 6. Instalación manual asistida (sin tiendas)

La extensión **no se distribuye por Chrome Web Store ni por AMO**. El flujo real:

1. El `.deb` instala los ZIP `segmenta-chromium.zip` y `segmenta-firefox.zip` en
   `/usr/share/segmenta/extensions/`.
2. La GUI (Preferencias → Navegadores) consulta `extension.status`, que detecta los navegadores
   instalados y reporta por cada uno `command`, `version`, `unsupported`, `manifest_present`,
   `user_manifest_present`, `extension_installed`, `prepared_path`, `archive_path` y un `state`
   (`unsupported`, `not-installed`, `extension-installed`, `native-ready`, `native-missing`).
3. **Preparar**: `extension.prepare` descomprime el ZIP en
   `$XDG_DATA_HOME/segmenta/extensions/<familia>-<versión>/` (se valida que existan `manifest.json` y
   `_locales`), listo para «Cargar descomprimida».
4. **Instalar manifiesto nativo**: `extension.install-native` escribe los manifiestos de usuario de
   los navegadores detectados.
5. **Abrir**: `extension.open` lanza el navegador con argv fijo en `chrome://extensions` (Chromium) o
   `about:debugging#/runtime/this-firefox` (Firefox); el usuario carga la carpeta descomprimida.
6. La CLI ofrece los mismos pasos con `segmenta extension status|prepare|open|install-native`.

Límites explícitos:

* Chrome estable **no** permite instalar una extensión sin tienda ni política de empresa; la GUI sólo
  prepara la carpeta y abre la página.
* Firefox sin firma se carga temporalmente desde `about:debugging`.
* Los navegadores cuyo binario vive en `/snap/` o `/flatpak/` se reportan como `unsupported`
  (`snap`, `flatpak`) porque no pueden ejecutar el host nativo; no se intenta burlar el sandbox.
* **(0.2, en preparación)** la tarea E1 mejora textos/estado de estos pasos y documenta los límites
  en la GUI; no añade tiendas ni firma.

---

## 7. Matriz de navegadores y alcance

| Navegador | Extensión | Manifiesto nativo de usuario |
| --- | --- | --- |
| Chrome | MV3 con `key` fija | `${XDG_CONFIG_HOME}/google-chrome/NativeMessagingHosts/io.github.wilkin.segmenta.json` |
| Chromium | MV3 | `${XDG_CONFIG_HOME}/chromium/NativeMessagingHosts/...` |
| Brave | MV3 | `${XDG_CONFIG_HOME}/BraveSoftware/Brave-Browser/NativeMessagingHosts/...` |
| Edge | MV3 | `${XDG_CONFIG_HOME}/microsoft-edge/NativeMessagingHosts/...` |
| Vivaldi | MV3 | `${XDG_CONFIG_HOME}/vivaldi/NativeMessagingHosts/...` |
| Opera | MV3 | `${XDG_CONFIG_HOME}/opera/NativeMessagingHosts/...` |
| Firefox | WebExtension con `browser_specific_settings.gecko.id = segmenta@wilkin.github.io` | `~/.mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json` |

* El host es común: recibe del navegador su propio manifiesto y no tiene lógica por navegador.
* **Sniffing fuera de YouTube no entra en 0.2**: no se prometen adaptadores nuevos. La captura
  genérica de descargas (§5) sí cubre cualquier sitio, porque opera sobre `chrome.downloads`, no sobre
  la página.
* Contexto de enlace: el menú contextual `segmenta-add-torrent` aparece en enlaces `magnet:*`,
  `http://*/*.torrent` y `https://*/*.torrent` y sólo llama a `torrent.add`.
