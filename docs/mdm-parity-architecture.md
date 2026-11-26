# Arquitectura de Paridad Funcional con MDM (Solo Linux / Ubuntu)

> **Documento:** ARCH-MDM-PARITY-02  
> **Línea base auditada:** Segmenta `v0.7.0` (`main` @ `9da3cf7`) frente a [`anefzaoui/misterdownloadmanager`](https://github.com/anefzaoui/misterdownloadmanager) (MDM)  
> **Alcance:** Exclusivamente funcional para **Ubuntu / Linux (GTK4 + Libadwaita + systemd)**. La portabilidad multiplataforma (Windows, macOS, RPM, AppImage, compilación ARM64, modo portable USB, 87 idiomas/RTL) queda **explícitamente fuera de alcance** por decisión de producto.

---

## Resumen ejecutivo

**Segmenta no parte de cero.** Entre `v0.4.0` y `v0.7.0`, el proyecto cerró la primera oleada de paridad prevista en [OpenSpec 13](openspec/13-mdm-parity-and-advanced-roadmap.md): incorporó servidor MCP por stdio, base de Debrid, Click'n'Load 2, paquetes en SQLite, extracción básica, `gallery-dl`, kill switch VPN sobre HTTP, lector RSS HTTP, popup reciente en extensiones, control remoto HTTP con QR e inhibición de suspensión.

Sin embargo, la auditoría directa contra el árbol actual de MDM revela dos frentes de ingeniería distintos:

1. **Capacidades ya presentes en Segmenta pero implementadas en versión acotada**, que requieren maduración arquitectónica para alcanzar la robustez y experiencia de usuario de MDM (VPN para torrents y herramientas externas, control remoto HTTPS con interfaz móvil, Debrid multi-cuenta y torrents en la nube, extracción con verificación CRC y contraseñas, RSS para torrents, video en vivo y galerías paralelas).
2. **Capacidades ausentes por completo en Segmenta**, que MDM ya resuelve en producción (suplantación TLS con BoringSSL, FTP/FTPS, DNS cifrado DoH/DoT, extractores nativos de nubes/hosters, Link Grabber con rastreo de carpetas y preview parcial de ZIP remoto, reglas por dominio, GeoIP/latencia y diagnóstico reproducible).

La ventaja estructural de Segmenta se mantiene como principio rector: todas las mejoras deben respetar la separación **daemon (`segmenta-daemon`) + clientes delgados (`segmenta-gui`, `segmenta-cli`, `segmenta-mcp`, `segmenta-host`)**, evitando acoplar lógica de red o persistencia al proceso de interfaz gráfica.

---

## Mapa rápido de brechas (Linux / Funcional)

| Dominio | Estado en Segmenta (`v0.7.0`) | Nivel en MDM | Brecha principal |
|---|---|---|---|
| **Motor HTTP y límites** | 1–32 hilos, `hot-split` al terminar, `posix_fallocate` + `pwrite` | Hasta 128 hilos, `HostLimits` adaptativo, evicción de workers lentos, *Site Rules*, PAC, *endless keep* | Media-Alta |
| **Anti-Bot / TLS** | *Seam* `tls_profile` (`chrome` devuelve error explícito) | BoringSSL (`browserlike.rs`) + escalera de fallback por sitio | Crítica |
| **VPN y aislamiento** | `SO_BINDTODEVICE` solo en `segmenta-core`; torrents bloqueados; `yt-dlp`/`gallery-dl` sin túnel | Todos los sockets de torrent atados + relay proxy interno `127.0.0.1` para `yt-dlp`/`gallery-dl`/`ffmpeg` + DoH/DoT por túnel | Crítica |
| **Protocolos** | HTTP/HTTPS + BitTorrent | HTTP/HTTPS + BitTorrent + **FTP/FTPS** con pooling y `REST`/`RETR` | Media |
| **DNS integrado** | DNS del sistema operativo | DoH / DoT (Cloudflare, Google, Quad9, AdGuard, NextDNS, propio) enrutado por la interfaz VPN | Media |
| **Extractores nativos** | Ninguno (URL directa, `yt-dlp`, `gallery-dl`, Debrid) | Google Drive (y Docs), MediaFire, Rapidgator, Gofile, LuluStream, Archive, Hugging Face (LFS SHA-256) y GitHub Releases | Alta |
| **Link Grabber y ZIP remoto** | Alta manual, `CNL2` y `packages` básicos en sidebar (sin filtrar la lista) | Link Grabber visual, lotes numerados, spider HTTP/FTP recursivo y **ZIP preview/descarga parcial por Range HTTP** | Alta |
| **Debrid** | Real-Debrid y AllDebrid por token manual, HTTP en alta GUI | 5 proveedores, OAuth/Device Code, multi-cuenta con fallback y **torrents/magnets descargados vía nube Debrid** | Media-Alta |
| **Torrents** | `librqbit` básico; prioridad secuencial parcial (`FileStream`) | Selección de árbol de archivos en GUI, blocklists IP `.dat`, trackers públicos, bind VPN, streaming en reproductor y creación CLI | Alta |
| **Control remoto** | API JSON en **HTTP plano**, 1 token, pausar/reanudar/cancelar | **HTTPS autofirmado** (fingerprint SHA-256), UI web móvil embebida, multidispositivo y alta de URLs/magnets/`.torrent` desde el móvil | Alta |
| **Video y Galerías** | VOD `yt-dlp`, `gallery-dl` síncrono (tope 110 s, sin GUI ni dedup) | Live streams con remux seguro a MP4, subtítulos/capítulos, recorte `--part`, cookies de navegador, galerías paralelas en motor propio y auto-update OpenPGP | Alta |
| **RSS** | Solo ítems HTTP/HTTPS; sin edición en GUI ni error persistido | Ítems `.torrent` y `magnet:`, edición y estado persistente | Media |
| **Archivos comprimidos** | `7z`/`unzip`, 1 clave vía env var, sin verificación CRC | Verificación post-descarga hasta 4 GB con **reparación parcial de rangos ZIP dañados**, lista MRU de claves en keyring y Papelera XDG | Media-Alta |
| **Extensión y escritorio** | Panel YouTube, captura HTTP y popup reciente; Firefox Snap bloqueado | Botón flotante sobre cualquier `<video>`, tab *"This page"* (medios/imágenes), envío a remoto, progreso en Dock Unity/D-Bus, *Quiet Mode* y RTC Wake | Media |

---

## Parte 0: Deuda interna previa (Prerrequisitos bloqueantes)

Antes de introducir módulos nuevos sobre el ecosistema de Segmenta, la evidencia registrada en [`odd/tasks/segmenta-v0.7.0-remediacion-gui.md`](../odd/tasks/segmenta-v0.7.0-remediacion-gui.md) (L5 y L6) expone dos defectos estructurales que deben resolverse como Fase 0:

1. **Aislamiento hermético de pruebas de GUI:** Las pruebas integradas de `segmenta-gui` pudieron alcanzar el socket real del daemon de usuario (`$XDG_RUNTIME_DIR/segmenta/daemon.sock`) y ejecutar acciones destructivas sobre la base productiva. Toda prueba de `segmenta-gui`, `segmenta-cli` y `segmenta-host` debe exigir un directorio efímero explícito (`SEGMENTA_SOCKET`, `XDG_RUNTIME_DIR` y `XDG_DATA_HOME` aislados) e impedir conexión al socket por defecto bajo perfil de test.
2. **Regresión `SIGSEGV` en `tests/prefs_pages.rs`:** La suite de Preferencias falla en `gtk_window_set_application` al construir `AdwApplicationWindow` y permanece `#[ignore]`. Sin cobertura automatizada de Preferencias no es seguro añadir páginas complejas (Multi-Debrid, DoH, *Site Rules*, dispositivos remotos y claves de archivo).

---

## Parte I: Mejorar lo que ya existe en Segmenta

### 1. Motor HTTP y control adaptativo (`segmenta-core`)

**Estado actual en Segmenta:**
- [`crates/segmenta-core/src/engine.rs`](../crates/segmenta-core/src/engine.rs) implementa planificación limpia con cobertura exacta, escritura directa con `posix_fallocate` + `pwrite`, *hot-split* cooperativo cuando un worker finaliza (`MIN_SPLIT_SIZE = 2 MiB`) y tope rígido `1..=32` conexiones ([`store.rs`](../crates/segmenta-daemon/src/store.rs#L756)).
- No existe memoria de límites por host, evicción proactiva de workers degradados, reglas por dominio ni conservación manual de streams HTTP sin tamaño conocido.

**Diseño objetivo (nivel MDM):**
- **Tabla `HostLimits` con decaimiento temporal:** Si un dominio responde `429`, `503` o cierra conexiones al abrir múltiples rangos paralelos, registrar el límite máximo observado para ese host en memoria/SQLite con TTL decreciente (`(auto)`), reduciendo hilos automáticamente en lugar de reintentar en bucle hasta agotar `max_retries`.
- **Evicción de workers lentos (`slow-worker eviction`):** Además de dividir cuando un hilo termina, el coordinador de `engine.rs` debe comparar la tasa móvil de cada worker contra la mediana del resto. Si un worker cae por debajo de un umbral sostenido y hay más de `MIN_SPLIT_SIZE` pendiente, cancelar ese socket, reencolar su tramo restante y abrir una conexión nueva limpia.
- **Tope configurable hasta 128 + `SiteRules` por patrón:** Elevar el máximo técnico a 128 conexiones en `EngineConfig` (manteniendo 16 por defecto) únicamente combinable con `HostLimits` y una nueva entidad `site_rules(domain_pattern, user_agent, custom_headers_secret_ref, max_connections, impersonate_tls)`.
- **Fallback *sparse* y captura de streams sin fin (*Keep*):** Permitir archivo *sparse* temporal cuando `posix_fallocate` no esté soportado o supere un presupuesto de latencia (p. ej. ciertos montajes FUSE/exFAT en Linux), y añadir acción `download.keep` para cerrar limpiamente descargas `TransferMode::SingleStream { total: None }` (radios, streams continuos) conservando el archivo recibido como `completed`.
- **Resolución PAC / WPAD (`proxy_mode`):** Extender `HttpConfig.proxy` para evaluar scripts `.pac` cuando el entorno de escritorio configure proxy automático.

---

### 2. Red privada, Kill Switch real y DNS cifrado (`netguard`, `torrent`, `tools`)

**Estado actual en Segmenta:**
- [`netguard.rs`](../crates/segmenta-core/src/netguard.rs) valida la interfaz (`ensure_up`) y [`probe.rs`](../crates/segmenta-core/src/probe.rs) aplica `SO_BINDTODEVICE` en los clientes `reqwest`.
- Cuando `vpn_interface` está configurada, [`manager.rs`](../crates/segmenta-daemon/src/manager.rs) rechaza explícitamente los torrents (`ERR_VPN_TORRENT_UNSUPPORTED`), mientras que `yt-dlp`, `gallery-dl`, `ffmpeg` y la resolución DNS del sistema operan fuera del control de `SO_BINDTODEVICE`.

**Diseño objetivo (nivel MDM):**
- **Relay Loopback autenticado para subprocesos (`crates/segmenta-daemon/src/vpn_relay.rs`):**
  - Inspirado en `crates/tools/src/relay.rs` de MDM: cuando `vpn_interface` no está vacía, el daemon levanta un proxy HTTP/SOCKS5 efímero en `127.0.0.1:<puerto_libre>` protegido con credenciales aleatorias por sesión.
  - Todo socket saliente abierto por ese relay ejecuta `SO_BINDTODEVICE` hacia `vpn_interface` (y revalida `ensure_up` antes de cada conexión).
  - `media.rs` (`yt-dlp`, `ffmpeg`), `gallery.rs` (`gallery-dl`) y `rss.rs` reciben `--proxy http://user:pass@127.0.0.1:<port>` y variables `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` limpiando el entorno para evitar bypass. Si la VPN cae, el relay cierra todas las conexiones activas al instante (*fail-closed*).
- **Torrents atados a la interfaz VPN:**
  - Configurar los sockets de `librqbit` (listener TCP/UDP, conexiones a pares, DHT y trackers) enlazados a la interfaz o dirección IPv4/IPv6 de `vpn_interface`, desactivando automáticamente UPnP, NAT-PMP y Local Peer Discovery (LPD) mientras el enlace VPN esté activo.
  - Al caer `vpn_interface`, pausar de inmediato las sesiones torrent con código de estado explícito y reanudarlas al recuperarse la interfaz.
- **Resolvedor DNS cifrado integrado (DoH / DoT):**
  - Extender [`urlssrf.rs`](../crates/segmenta-core/src/urlssrf.rs) y la construcción de `Session` para soportar ajuste `dns_mode` (`system` | `doh` | `dot`) y proveedores preconfigurados (Cloudflare, Quad9, Google, AdGuard, NextDNS o endpoint propio).
  - Las consultas DoH/DoT se ejecutan desde sockets ligados al mismo `vpn_interface`, eliminando fugas DNS (*DNS leaks*) al usar VPN, sin perder la validación anti-SSRF sobre las IPs resueltas.

---

### 3. Debrid completo (`segmenta-core/src/debrid.rs` + `daemon` + `prefs.rs`)

**Estado actual en Segmenta:**
- [`debrid.rs`](../crates/segmenta-core/src/debrid.rs) soporta `Real-Debrid` y `AllDebrid` (`TorBox` retorna `ERR_DEBRID_UNSUPPORTED`), con token único por proveedor en el llavero (`secrets.rs`), configurado en Preferencias y resuelto únicamente desde [`add_dialog.rs`](../crates/segmenta-gui/src/add_dialog.rs#L579).

**Diseño objetivo (nivel MDM):**
- **Cinco proveedores completos:** Implementar `TorBox`, `Premiumize` y `Mega-Debrid` en `segmenta-core/src/debrid.rs`, además de `Real-Debrid` y `AllDebrid`.
- **Autenticación fluida (Device Code / OAuth) + metadata:** Incorporar el flujo interactivo de emparejamiento en navegador (código corto en GUI/CLI consultando polling de autorización) y persistir estado de cuenta (`username`, `is_premium`, `expires_at` vía `debrid.status`), sin almacenar jamás secretos fuera del keyring.
- **Múltiples cuentas ordenables por prioridad:** Nueva tabla `debrid_accounts(id, provider, label, priority, enabled)` en SQLite referenciando su secreto en keyring (`debrid:<account_id>`). Al resolver una URL, el daemon recorre las cuentas habilitadas en orden de prioridad hasta obtener un enlace directo válido.
- **Resolución centralizada en el Daemon:** Mover el seam `debrid_link` desde `add_dialog.rs` hacia `manager.rs` (con flag de bypass opcional `direct: true`) para que CLI, extensión del navegador, Link Grabber y MCP también desbloqueen enlaces automáticamente.
- **Torrents/Magnets remotos a través de Debrid:** Permitir la opción *"Descargar torrents a través de cuenta Debrid"* (`torrent_via_debrid`): el daemon envía el magnet/info-hash al proveedor Debrid, consulta progreso remoto y, al completarse en la nube del proveedor, descarga los archivos resultantes mediante `segmenta-core` (HTTP multi-hilo + keyring + VPN), limpiando el trabajo remoto al terminar.

---

### 4. Control remoto HTTPS con interfaz web móvil (`crates/segmenta-daemon/src/remote.rs`)

**Estado actual en Segmenta:**
- [`remote.rs`](../crates/segmenta-daemon/src/remote.rs) expone `GET /api/downloads` y `POST /api/action` (`pause`/`resume`/`cancel`) en **HTTP sin cifrar**, con un único token revocable y código QR dibujado en `remote_dialog.rs` y CLI.

**Diseño objetivo (nivel MDM):**
- **TLS local autofirmado obligatorio:** Generar un par de claves + certificado X.509 local autofirmado en la carpeta de datos del daemon (cubriendo la IP LAN y `localhost`, renovable al expirar o cambiar interfaz), servir el servidor remoto con `tokio-rustls` sobre **HTTPS** y mostrar su huella **SHA-256** en `remote_dialog.rs` y `segmenta remote status`.
- **Frontend web móvil embebido (`crates/segmenta-daemon/remote-web/`):** Incluir vía `include_str!` / `rust-embed` una SPA/PWA minimalista (adaptada a claro/oscuro e idiomas `es`/`en`/`pt-BR`) con:
  - Barra de estado global (velocidad total, espacio libre en el directorio de descargas).
  - Alta remota: pegar URL HTTP/HTTPS, enlace `magnet:` o subir archivo `.torrent` desde el teléfono, eligiendo cola y si inicia de inmediato.
  - Filtros (`Todas`, `Activas`, `Completadas`), acciones por fila (`Pausar`, `Reanudar`, `Eliminar`) y control de arranque/parada de colas.
- **Registro multidispositivo con emparejamiento temporal:** Reemplazar el único alias `remote-session-token` por tabla SQLite `remote_devices(id, name, token_hash, paired_at, last_seen_at)` + códigos efímeros de emparejamiento de 8 dígitos (TTL 10 min, máx. 5 intentos con *rate limiting*). Permitir ver y revocar dispositivos individuales en `remote_dialog.rs` sin desconectar a los demás.
- **Modo servidor dedicado:** Documentar e integrar soporte de systemd/headless (`segmenta-daemon` en servidor doméstico o NAS sin X11/GUI, emparejando vía `segmenta remote pair`).

---

### 5. Extracción, verificación de integridad y reparación de archivos (`extract.rs`)

**Estado actual en Segmenta:**
- [`extract.rs`](../crates/segmenta-daemon/src/extract.rs) soporta `zip`, `7z`, `rar` y volúmenes `.001` vía `7z`/`unzip`, protege frente a rutas absolutas/`..` y symlinks en staging, y lee una sola contraseña de entorno (`SEGMENTA_EXTRACT_PASSWORD`).

**Diseño objetivo (nivel MDM):**
- **Verificación de integridad al completar (`check`):** Antes de marcar un archivo `.zip`, `.7z`, `.rar` como finalizado (hasta un umbral configurable, ej. 4 GiB por defecto, o bajo demanda desde el menú contextual *"Comprobar archivo"*), validar su CRC/estructura (`zip` en Rust puro con `zip` crate o `7z t` sin escribir a disco).
- **Reparación parcial de ZIP dañados:** Conservar temporalmente las cabeceras de validación (`ETag`/`Last-Modified` y tabla de segmentos) si la comprobación automática de un `.zip` detecta entradas corruptas: calcular los rangos de bytes exactos afectados dentro del ZIP y re-descargar únicamente esos tramos con `If-Range` antes de recurrir a una re-descarga completa.
- **Gestor de contraseñas en llavero (MRU) y diálogo interactivo:** Reemplazar la variable de entorno estática por lista de contraseñas guardadas en `secrets.rs` (Secret Service), ordenadas por uso más reciente; si ninguna abre el archivo, pausar la fase de extracción emitiendo evento a la GUI para solicitar la contraseña al usuario (con opción de guardarla solo si logra extraer).
- **Política de carpeta y Papelera XDG:** Añadir opciones explícitas en Preferencias para controlar dónde van los archivos extraídos (subcarpeta siempre / solo si contiene múltiples entradas / carpeta actual), soporte opcional *in-place* si está disponible `hatchzip`, y mover el archivo original a la **Papelera** del escritorio (`org.freedesktop.FileManager1` / `gio trash`) cuando `extract_remove_source` está activo en lugar de borrado definitivo.

---

### 6. Torrents avanzados (`crates/segmenta-daemon/src/torrent.rs`)

**Estado actual en Segmenta:**
- Sondeo básico de metadatos (`probe_metainfo` con listado de `files`), flag `torrent_sequential` mediante `FileStream` en `librqbit` (prioridad inicial), bloqueo bajo VPN y sin selector de archivos en GUI.

**Diseño objetivo (nivel MDM):**
- **Selección de archivos en diálogo de alta:** Cuando `torrent.probe` o un magnet resuelve su árbol de archivos, mostrar en GUI un árbol seleccionable con tamaños acumulados, carpeta contenedora opcional y verificación de colisión de rutas con tareas existentes.
- **Seguridad y privacidad P2P:**
  - Descarga automática cada 7 días y aplicación de **blocklists de IPs** (PeerGuardian / eMule `.dat`, plano o `.gz`).
  - Interruptor opcional para inyectar una lista verificada de **trackers públicos** únicamente a torrents públicos (nunca a privados).
  - **Propiedad estricta de archivos en disco:** No escribir ni compartir sobre archivos preexistentes en disco que Segmenta no creó, solicitando confirmación explícita si un archivo en la ruta destino fue reemplazado externamente.
- **Streaming real en GUI y CLI avanzada:** Botón *"Reproducir mientras descarga"* en filas torrent de audio/video (activa modo secuencial/priorización continua del stream y abre `xdg-open`), más subcomando `segmenta torrent create` para generar `.torrent` desde terminal.

---

### 7. Medios (`yt-dlp`), Galerías (`gallery-dl`) y RSS (`media.rs`, `gallery.rs`, `rss.rs`)

**Estado actual en Segmenta:**
- `media.rs` descarga VOD/playlists públicos; `gallery.rs` ejecuta `gallery-dl` síncronamente con timeout fijo de 110 s y sin vista en GUI ni deduplicación; `rss.rs` encola solo URLs HTTP/HTTPS.

**Diseño objetivo (nivel MDM):**
- **Video avanzado (`media.rs`):**
  - **Grabación segura de Live Streams:** Permitir grabar directos y convertir `.ts` a `.mp4` al pulsar *"Detener y guardar"*, verificando duración de video/audio con `ffprobe` antes de borrar el `.ts` intermedio y renombrando grabaciones previas (`(grabación 1).ts`) para no sobrescribir al reanudar.
  - **Subtítulos, capítulos y recorte parcial (`--part`):** Permitir seleccionar subtítulos (embebidos o externos `.vtt`/`.srt`), incrustar capítulos, descargar rangos de tiempo (`start-end`) y pasar cookies opcionales desde perfiles de navegador locales (`--cookies-from-browser`) ante bloqueos *"Sign in to confirm you're not a bot"*.
  - **Auto-actualización verificada de herramientas (`yt-dlp`, `ffmpeg`, `gallery-dl`):** Mecanismo en segundo plano (GUI y CLI `segmenta tools update`) que actualiza binarios en `/usr/share/segmenta/bin` o `~/.local/share/segmenta/bin` validando firma **OpenPGP / SHA-256 fijado** contra los releases oficiales.
- **Galerías paralelas y reanudables (`gallery.rs`):**
  - Desacoplar la extracción de URLs de la descarga: ejecutar primero `gallery-dl --dump-json` (con timeout acotado) para obtener la lista de imágenes/medios, crear un paquete en SQLite y bajar las imágenes en paralelo (p. ej. 4 simultáneas) usando `segmenta-core` (respetando límite global de velocidad, VPN relay y barra de progreso en vivo).
  - Usar `gallery-dl` directo solo como fallback para ítems que requieran cookies/sesión interna y permitir **"Reanudar galería"** omitiendo archivos ya presentes en carpeta.
- **RSS completo (`rss.rs` + `rss_panel.rs`):**
  - Admitir enlaces `.torrent` y `magnet:` detectados en `<item>`/`<enclosure>`/`<entry>` creando tareas BitTorrent filtradas, agregar edición de feeds existentes en [`rss_panel.rs`](../crates/segmenta-gui/src/rss_panel.rs) y persistir `last_error` y `last_error_at` en `rss_feeds`.

---

### 8. Extensión de navegador, paquetes, lista y automatización (`extension`, `gui`, `mcp`, `cli`)

**Diseño objetivo para cerrar brechas de flujo diario:**
- **Extensión de navegador (`extension/shared/`):**
  - Botón flotante arrastrable sobre cualquier `<video>` HTML5 en cualquier página (extendiendo el patrón actual exclusivo de YouTube).
  - Pestaña **"Esta página"** en el popup mostrando todos los medios e imágenes detectados en la pestaña activa para descarga rápida + interruptor por dominio *"No capturar nunca en este sitio"*.
  - Soporte de **formularios `POST`** interceptados, aislamiento de cookies/historial para descargas originadas en **ventanas privadas/incógnito** y opción de enviar descargas a una instancia remota de Segmenta.
  - Resolver la compatibilidad de Native Messaging con **Firefox Snap** en Ubuntu (evaluando puente vía portal XDG / wrapper documentado).
- **GUI Libadwaita y Lista (`segmenta-gui`):**
  - **Filtrado real por paquetes** en la barra lateral + agrupación visual colapsable (`adw::ExpanderRow`) en la lista principal.
  - **Redownload seguro y Move/Rename:** Al re-descargar un archivo completado, escribir a un archivo temporal nuevo y sustituir atómicamente el anterior solo al llegar al 100% limpio; añadir *"Mover / Renombrar"* sincronizando ruta en SQLite y disco.
  - **Integración escritorio Ubuntu:** Reportar barra de progreso global al Dock de GNOME/Ubuntu vía señal D-Bus `com.canonical.Unity.LauncherEntry` (`LauncherEntry`) y agregar **Modo Silencioso (*Quiet Mode*)** detectando bloqueo de sesión o app a pantalla completa antes de levantar diálogos modales de confirmación.
- **Planificador (`scheduler.rs`):**
  - **Despertar del sistema (RTC Wake):** Integrar programación de despertar antes del inicio de una cola (mediante `systemd` timer `WakeSystem=true` o `rtcwake` bajo polkit/capabilidad limpia).
  - **Cuotas de transferencia por ventana horaria:** Límite de GB descargados cada N horas (pausando automáticamente y reanudando al renovarse la cuota).
- **Paridad de CLI y MCP (`segmenta-cli`, `segmenta-mcp`):**
  - Ampliar [`segmenta-mcp`](../crates/segmenta-mcp/src/main.rs) de las 5 herramientas actuales (`list`, `add`, `pause`, `resume`, `delete`) hacia gestión de **colas, paquetes, reintentos, sondeo de medios/torrents y verificación RSS**, manteniendo esquema JSON-RPC estricto y autorización por UID.
  - Agregar streaming JSONL en tiempo real en `segmenta` CLI para monitoreo por scripts (`download.events`).

---

## Parte II: Módulos nuevos faltantes en Segmenta

Los siguientes bloques no tienen equivalente implementado hoy en `crates/` y representan nuevas capacidades a diseñar en crates dedicados o módulos de `segmenta-core`/`segmenta-daemon`:

### Módulo A: Suplantación TLS / Evitación Anti-Bot (`feature impersonate`)

- **Problema:** Muchos hosters y CDNs (Cloudflare, FileHippo, etc.) bloquean el *ClientHello* estándar de `rustls` por análisis de huella JA3/JA4, devolviendo `403`/`412` antes incluso de validar cabeceras.
- **Referencia en MDM:** `crates/engine/src/browserlike.rs`, `tls.rs`.
- **Arquitectura recomendada para Segmenta:**
  - Implementar la ruta documentada en [`docs/tls-impersonation-evaluation.md`](tls-impersonation-evaluation.md): añadir **`rquest`** (fork de `reqwest` sobre BoringSSL con emulación de perfiles Chrome/Firefox) detrás de la feature opcional `impersonate` en `segmenta-core` (habilitable en el build oficial del `.deb` de Ubuntu una vez validado el tiempo de compilación en CI con CMake/Clang).
  - Incorporar la **escalera de reintentos inteligente** de MDM en `probe.rs` sin resolver CAPTCHAs ni eludir desafíos interactivos:
    1. Petición inicial (`standard` o modo navegador según `SiteRules`).
    2. Si el servidor rechaza con `403`/`412` típico de fingerprinting, reintentar con perfil de navegador + orden real de cabeceras y cookies de la extensión.
    3. Si un servidor rechaza suplantación de navegador y permite clientes declarados (caso típico de mirrors como SourceForge), caer una vez a `Segmenta/<version>` recordándolo en caché de sesión.

### Módulo B: Cliente FTP y FTPS segmentado (`segmenta-core/src/ftp.rs`)

- **Problema:** Segmenta rechaza `ftp://` y `ftps://`.
- **Referencia en MDM:** `crates/engine/src/ftp.rs`.
- **Arquitectura recomendada para Segmenta:**
  - Reutilizar `planner::SegmentRange`, `DiskWriter` (`pwrite`) y `HostLimits` con un adaptador FTP/FTPS asíncrono (`suppaftp` o cliente Tokio TLS):
    - Sondeo inicial con `SIZE` + `MDTM` (validación equivalente a `ETag`/`Last-Modified` al reanudar).
    - Pool de sesiones de control por descarga con `REST <offset>` + `RETR` en modo pasivo (`PASV`/`EPSV`, más opción activa), pasando por proxy SOCKS5 o `vpn_interface` (`SO_BINDTODEVICE`) cuando estén activos.

### Módulo C: Extractores nativos de Nubes, Hosters y Repositorios (`segmenta-core/src/sites/` y `hub.rs`)

- **Problema:** Enlaces de Google Drive, MediaFire, Gofile o repositorios de modelos/releases requieren múltiples saltos HTML, formularios de confirmación o paginación API que hoy fallan como descarga HTTP cruda o bajan páginas `.html`.
- **Referencia en MDM:** `crates/engine/src/sites/*`, `crates/engine/src/hub.rs`.
- **Arquitectura recomendada para Segmenta:**
  - **Módulo `sites/`:** Funciones puras y testeadas sin red (contra servidor de pruebas local) para extraer la URL real de archivo y manejar enlaces caducados en:
    - **Google Drive** (confirmación de virus scan en archivos grandes, exportación automática de Docs/Sheets/Slides a formatos Office/PNG, uso de cookies de extensión para archivos restringidos).
    - **MediaFire, Gofile, Rapidgator, Internet Archive, LuluStream** (incluyendo reenvío acotado de formularios `application/x-www-form-urlencoded`).
  - **Módulo `hub.rs` (GitHub & Hugging Face):** Dado un enlace a un release/tag de GitHub o a un modelo/dataset de Hugging Face, consultar su API pública paginada (`Link: rel="next"`), validar rutas relativas estrictas (anti-traversal), poblar automáticamente los hashes **SHA-256** publicados (LFS / release assets) para verificación en `checksum.rs` y permitir token de `huggingface.co` vía `Site Logins` (keyring).

### Módulo D: Link Grabber, Spidering de Carpetas y ZIP Preview Remoto

- **Problema:** Añadir 50 enlaces desde un listado `"Index of /"` o extraer 1 archivo puntual de un `.zip` de 10 GB obliga hoy a pegar enlaces uno por uno o bajar todo el archivo.
- **Referencia en MDM:** `crates/engine/src/folder.rs`, `remotezip.rs`, `crates/app/src/grabber.rs`, `linkparts.rs`.
- **Arquitectura recomendada para Segmenta:**
  - **Motor de exploración en `segmenta-core`:**
    - `batch.rs`: expansión de patrones secuenciales (`archivo_[01-25].jpg`).
    - `folder.rs`: rastreo *breadth-first* acotado (`MAX_DEPTH`, `MAX_FOLDERS`, `MAX_FILES`) de listados HTTP `"Index of"` y directorios FTP (`MLSD`/`LIST`), filtrando subcarpetas padre y hosts externos.
    - `remotezip.rs`: lectura mediante peticiones `Range` del final del ZIP remoto (EOCD + ZIP64 + Central Directory) para listar su árbol interno en el diálogo de alta sin descargar el archivo completo, y descarga selectiva vía `Range` + `If-Range` + descompresión Deflate/Stored al vuelo con validación CRC-32.
  - **Diálogo Link Grabber en `segmenta-gui` + `LinksPart` en `segmenta-host`:**
    - Ventana previa al encolado donde llegan listas capturadas desde portapapeles, extensión (*chunked* bajo el tope de 1 MiB de Native Messaging), CNL2, carpetas rastreadas o Hugging Face/GitHub.
    - Comprobación en segundo plano de estado en línea, nombre real y tamaño (`link_lookup`), filtrado por tipo/texto, asignación directa a un Paquete y contraseña de extracción asociada al lote.

### Módulo E: GeoIP local, Latencia y Diagnóstico Reproducible (`segmenta-geo` / `segmenta doctor`)

- **Referencia en MDM:** `crates/geo`, `crates/crash`.
- **Arquitectura recomendada para Segmenta (enfoque Linux):**
  - **GeoIP offline + RTT:** Construir en tiempo de compilación/empaquetado una tabla compacta de rangos IP -> código ISO de país (DB-IP Lite, 100% offline, cero llamadas externas) y mostrar bandera/país + latencia del socket en la vista de detalle de descarga (`segment_view.rs` / página de detalle).
  - **Diagnóstico Linux (`segmenta system diagnose`):** Dado que en Ubuntu existe `systemd-coredump` y logs de journal, MDM gana en accionabilidad al empaquetar reportes listos para issues. Segmenta debe proveer un comando/botón *"Exportar informe de diagnóstico"* que genere un paquete de texto sanitizado (eliminando URLs completas, tokens, usuario Linux y cookies), incluyendo estado de SQLite `user_version`, últimas líneas del journal de `segmenta.service`, versión de `yt-dlp`/`ffmpeg`/`7z`, estado del socket e historial reciente de backtraces/panics de Tokio.

---

## Parte III: Hoja de ruta recomendada (Sin multiplataforma)

La evolución hacia paridad funcional completa con MDM conviene ejecutarla en 4 hitos secuenciales sobre `main`, conservando el modelo ODD y evitando regresiones:

| Fase | Objetivo central | Bloques incluidos |
|---|---|---|
| **Hito 0 (Prerrequisito)** | Estabilidad de pruebas y confianza en UI | Aislamiento hermético de sockets/DB en tests GUI + reparación de `SIGSEGV` en `tests/prefs_pages.rs`. |
| **Hito 1 (Red e Inmunidad)** | Cerrar las brechas críticas de transporte y VPN | Proxy Relay VPN para `yt-dlp`/`gallery-dl`, bind VPN en `librqbit`, DoH/DoT por VPN, feature opcional `rquest` (TLS Chrome) + escalera anti-bot, `HostLimits` adaptativo y evicción de workers lentos. |
| **Hito 2 (Captura, Hosters y Remoto)** | Productividad avanzada y experiencia móvil | Extractores `sites/` (GDrive, MediaFire, Gofile, Rapidgator) + `hub.rs` (GitHub/Hugging Face), FTP/FTPS, Link Grabber + Spidering + ZIP remoto parcial, Control Remoto HTTPS con PWA móvil y multidispositivo. |
| **Hito 3 (Debrid, Archivos y Torrents)** | Profundidad en descargas complejas | Multi-Debrid (5 proveedores + Device Code + torrents en Debrid), torrents con selección de archivos y blocklists IP, testeo CRC y reparación parcial de ZIP + claves MRU en keyring, RSS para torrents + edición GUI. |
| **Hito 4 (Medios, Extensión y Pulido Linux)** | Paridad de escritorio GNOME/Ubuntu | Live streams en `media.rs`, galerías `gallery-dl` paralelas sobre `segmenta-core`, auto-update firmado de helpers, botón flotante HTML5 en extensión + *"Esta página"*, filtro real por paquetes en GUI, progreso Dock Unity/D-Bus, *Quiet Mode*, RTC Wake, cuotas, GeoIP y expansión de tools en `segmenta-mcp`. |

---

## Checklist de arquitectura para futuras implementaciones

- [ ] Ningún secreto nuevo (tokens Debrid, claves de proxy, contraseñas de archivo, tokens Hugging Face o hashes remotos) se guarda en texto plano en SQLite o logs; todo pasa por `secrets.rs`.
- [ ] Todo binario externo (`yt-dlp`, `gallery-dl`, `ffmpeg`, `7z`) hereda la configuración de proxy/VPN relay y timeouts estrictos sin invocación por shell (`argv` puro).
- [ ] Ninguna prueba de integración o GUI apunta al socket ni a la base real del usuario en `$HOME/.local/share/segmenta`.
- [ ] Todo trabajo nuevo sobre estos módulos preserva compatibilidad aditiva de esquema SQLite (`user_version`) y contratos JSON-RPC documentados en `docs/openspec/`.

---

## Siguiente paso

Tomar el **Hito 0** como primer plan ODD en `odd/tasks/` antes de abrir cambios de motor o preferencias, garantizando que todas las pruebas de GUI corran aisladas y `prefs_pages.rs` recupere cobertura real sin `#[ignore]`.
