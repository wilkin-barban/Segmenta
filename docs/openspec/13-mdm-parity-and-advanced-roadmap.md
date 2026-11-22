# OpenSpec 13: Hoja de Ruta de Paridad Arquitectónica con Mister Download Manager (MDM)

> **Documento:** SPEC-013 / ARCH-ROADMAP  
> **Estado:** Propuesta Técnica Aprobada para Futuras Releases  
> **Proyecto de Referencia:** [Mister Download Manager (MDM)](https://github.com/anefzaoui/misterdownloadmanager) de Ahmed Nefzaoui (Licencia GPL-3.0, Rust)  
> **Objetivo:** Definir con rigor arquitectónico las capacidades ausentes en **Segmenta**, identificar los módulos fuente exactos de MDM de donde pueden adoptarse o portarse algoritmos y contratos, y establecer la guía técnica paso a paso para integrarlas en el ecosistema desacoplado de Segmenta (`core`, `daemon`, `gui`, `cli`, `host`, `extension`).

---

## 1. Resumen Ejecutivo y Diagnóstico Comparativo

**Segmenta** cuenta con cimientos sólidos: motor de segmentación dinámica en Rust (`segmenta-core`), persistencia transaccional SQLite con WAL (`segmenta-daemon`), GUI nativa moderna GNOME/Libadwaita (`segmenta-gui`), CLI automatizable (`segmenta-cli`) y extensiones WebExtension Manifest V3 con protocolo Native Messaging framing (`segmenta-host`).

Sin embargo, el proyecto de código abierto **Mister Download Manager (MDM)** ha demostrado una evolución funcional superior en aspectos de automatización con IA, soporte para servidores multihoster (Debrid), captura masiva, integración de red con VPNs y utilidades de post-procesado automatizado.

La siguiente tabla resume las brechas funcionales detectadas y su mapeo de integración:

| Capacidad | Estado en Segmenta | Módulo en MDM (`anefzaoui/misterdownloadmanager`) | Crate Destino en Segmenta |
|---|---|---|---|
| **1. Servidor MCP para Asistentes de IA** | ❌ Ausente | `crates/mcp` (`mdm-mcp`) | `crates/segmenta-mcp` (nuevo crate) |
| **2. Cuentas y Servicios Debrid** | ❌ Ausente | `crates/debrid` (`mdm-debrid`) | `crates/segmenta-core/src/debrid/` |
| **3. Click'n'Load 2 y Paquetes** | ❌ Ausente | `crates/cnl` (`mdm-cnl`), `crates/app/src/linkparts.rs` | `crates/segmenta-daemon/src/cnl.rs` |
| **4. Descompresión Automática (Extract)** | ❌ Ausente | `crates/extract` (`mdm-extract`) | `crates/segmenta-daemon/src/postprocess/extract.rs` |
| **5. Descarga de Galerías (`gallery-dl`)** | ❌ Solo yt-dlp | `crates/tools` (`mdm-tools`) | `crates/segmenta-daemon/src/media.rs` |
| **6. Atado a Interfaz VPN y Kill Switch** | ❌ Ausente | `crates/tools/src/relay.rs`, `crates/engine` | `crates/segmenta-core/src/engine.rs` |
| **7. Suplantación TLS / Anti-Bot (BoringSSL)** | ❌ Rustls/Reqwest estándar | `crates/engine` (BoringSSL bindings) | `crates/segmenta-core/src/probe.rs` |
| **8. DNS Cifrado (DoH - DNS over HTTPS)** | ❌ DNS del sistema | `crates/engine` | `crates/segmenta-core/src/urlssrf.rs` |
| **9. Torrent Secuencial y Streaming** | ⚠️ Parcial | `crates/torrent` (`librqbit` sequential flags) | `crates/segmenta-daemon/src/torrent.rs` |
| **10. Canales RSS para Torrents/Descargas** | ❌ Ausente | `crates/app/src/rss.rs` | `crates/segmenta-daemon/src/scheduler.rs` |
| **11. Popup de Descargas en la Extensión** | ⚠️ Solo panel | `extension/`, `crates/nmhost` (`RecentAsk`) | `extension/shared/popup.js`, `crates/segmenta-host` |
| **12. Control Remoto Web y Modo Headless** | ❌ Solo local IPC | `crates/remote`, `crates/app/src/headless.rs` | `crates/segmenta-daemon/src/remote.rs` |
| **13. Geolocalización IP y Banderas** | ❌ Ausente | `crates/geo` (`mdm-geo`) | `crates/segmenta-gui/src/segment_view.rs` |
| **14. Inhibición de Suspensión (D-Bus)** | ❌ Ausente | `crates/app/src/unix.rs` | `crates/segmenta-daemon/src/scheduler.rs` |

---

## 2. Especificación Detallada de Mejoras Punto por Punto

---

### Módulo 1: Servidor MCP para Asistentes de IA (Model Context Protocol)

#### 1.1. Justificación y Alcance
Permite que entornos como **Gentle Dot**, Claude Desktop, Cursor o agentes autónomos interactúen de forma segura y directa con Segmenta sin requerir comandos de terminal ni interfaces gráficas.

#### 1.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/mcp/`
- **Archivos clave:**
  - `crates/mcp/src/main.rs`: Inicialización del protocolo stdio JSON-RPC compatible con el estándar MCP de Anthropic.
  - `crates/mcp/src/tools.rs`: Definición de herramientas expuestas: `download_list`, `download_add`, `download_pause`, `download_resume`, `download_remove`.

#### 1.3. Implementación en Segmenta
1. **Creación del Crate:** Crear `crates/segmenta-mcp/Cargo.toml` con dependencia hacia `rmcp` o serializador JSON-RPC manual sobre `tokio::io::stdin()` / `stdout()`.
2. **Conexión IPC:** El binario `segmenta-mcp` actúa como cliente ligero del socket Unix existente (`/run/user/$UID/segmenta.sock`).
3. **Herramientas a exponer:**
   - `segmenta_list`: Consulta `task.list` con filtros de estado (`active`, `completed`, `failed`).
   - `segmenta_add`: Ejecuta `task.create` enviando la URL, headers opcionales y carpeta de destino.
   - `segmenta_action`: Envía `task.pause`, `task.resume`, `task.retry` o `task.delete`.
4. **Instalación:** Registrar automáticamente el manifiesto MCP en `~/.config/Claude/claude_desktop_config.json` o mediante autodescubrimiento en asistentes compatibles.

---

### Módulo 2: Soporte para Servicios y Cuentas Debrid

#### 2.1. Justificación y Alcance
Los servicios multihoster (Debrid) permiten a los usuarios convertir enlaces de alojamiento lento (Rapidgator, 1fichier, Turbobit, Mega, etc.) en enlaces de descarga directa sin límites de velocidad ni esperas.

#### 2.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/debrid/`
- **Archivos clave:**
  - `crates/debrid/src/realdebrid.rs`: Implementación del flujo de desbloqueo de URLs (`/unrestrict/link`) y autenticación API token.
  - `crates/debrid/src/alldebrid.rs`, `torbox.rs`, `premiumize.rs`: Adaptadores para cada API de proveedor.
  - `crates/debrid/src/lib.rs`: Trait unificado `DebridService` con método `resolve_link(url, credentials) -> Result<DirectUrl>`.

#### 2.3. Implementación en Segmenta
1. **Ubicación en Segmenta:** Nuevo módulo `crates/segmenta-core/src/debrid/`:
   ```rust
   pub trait DebridProvider: Send + Sync {
       async fn unrestrict(&self, api_key: &str, target_url: &str) -> Result<String, DebridError>;
       fn can_handle(&self, host: &str) -> bool;
   }
   ```
2. **Almacenamiento Seguro de Claves:** Reutilizar el backend existente `crates/segmenta-daemon/src/secrets.rs` (Secret Service en Linux) bajo la clave `segmenta:debrid:<provider_name>`.
3. **Intercepción en el Daemon:** En `crates/segmenta-daemon/src/manager.rs`, antes de enviar una URL a `probe()`, verificar si el dominio coincide con un host soportado por Debrid y si el usuario tiene una API key configurada; en caso afirmativo, resolver el enlace directo primero.
4. **Configuración en GUI:** En `crates/segmenta-gui/src/prefs.rs`, añadir una página dedicada "Cuentas Debrid" con campos para validar tokens mediante una llamada de prueba.

---

### Módulo 3: Captura Masiva Click'n'Load 2 y Link Grabber con Paquetes

#### 3.1. Justificación y Alcance
Click'n'Load es el estándar de facto en sitios de enlaces para enviar listas completas al gestor de descargas con un solo clic. Agruparlas en "Paquetes" evita saturar la interfaz de usuario con docenas de filas independientes.

#### 3.2. Dónde estudiarlo en MDM
- **Directorios:** `crates/cnl/` y `crates/app/src/linkparts.rs`
- **Archivos clave:**
  - `crates/cnl/src/server.rs`: Servidor HTTP local en `127.0.0.1:9666` escuchando rutas `/flash/add`, `/flash/addcrypted2`.
  - `crates/cnl/src/decrypt.rs`: Rutas de descifrado AES-128-CBC de cargas cifradas CNL2 (clave fija `1234567890123456` o derivada del handshake).
  - `crates/app/src/linkparts.rs`: Agrupador de URLs fragmentadas recibidas desde la extensión de navegador o CNL.

#### 3.3. Implementación en Segmenta
1. **Listener CNL en Daemon:** En `crates/segmenta-daemon/src/cnl.rs`, levantar un listener `axum` o `tokio` en `127.0.0.1:9666` restringido estrictamente a localhost y con validación de cabecera `Origin` o confirmación del usuario para mitigar ataques CSRF de sitios web maliciosos.
2. **Esquema de Base de Datos (`schema.sql`):**
   ```sql
   ALTER TABLE tasks ADD COLUMN package_id TEXT;
   CREATE TABLE packages (
       id TEXT PRIMARY KEY,
       name TEXT NOT NULL,
       destination TEXT NOT NULL,
       created_at INTEGER NOT NULL
   );
   ```
3. **Integración en Libadwaita (`segmenta-gui`):** En `crates/segmenta-gui/src/window.rs`, agrupar las filas que compartan `package_id` en un `AdwExpanderRow` colapsable que muestre el progreso acumulado y permita pausar o reanudar todo el paquete en bloque.

---

### Módulo 4: Descompresión Automática Multiformato (`mdm-extract`)

#### 4.1. Justificación y Alcance
Al descargar archivos comprimidos (especialmente descargas divididas en partes `.part1.rar`, `.z01`, etc.), el usuario espera que el archivo final quede listo para usar sin requerir herramientas externas manuales.

#### 4.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/extract/`
- **Archivos clave:**
  - `crates/extract/src/zip.rs`: Descompresión en proceso de `.zip`, `.tar.gz`, `.tar.xz`.
  - `crates/extract/src/sevenzip.rs`: Detección y ejecución desacoplada del binario `7z` (o `7zz` en Linux) para formatos propietarios `.rar` y `.7z`.
  - `crates/extract/src/passwords.rs`: Iteración sobre lista de contraseñas guardadas hasta encontrar la correcta.

#### 4.3. Implementación en Segmenta
1. **Ampliación de Post-Procesado:** Extender `crates/segmenta-daemon/src/postprocess.rs` incorporando un submódulo `extract.rs`.
2. **Pipeline de Tarea:**
   - Al alcanzar el estado `verifying` y validar el checksum (o completitud de partes), si la opción `auto_extract` está activa para esa tarea o categoría:
   - Identificar tipo MIME (`application/zip`, `application/x-rar-compressed`, `application/x-7z-compressed`).
   - Ejecutar la extracción en un hilo de trabajo seguro con `nice` bajo para no congelar el sistema.
   - Si la extracción es exitosa y el usuario lo habilitó, enviar el archivo comprimido original a la papelera mediante el protocolo `org.freedesktop.FileManager1.Trash` de XDG.

---

### Módulo 5: Descarga de Galerías de Imágenes (`gallery-dl`)

#### 5.1. Justificación y Alcance
Segmenta gestiona video mediante `yt-dlp`. Integrar `gallery-dl` añade soporte instantáneo para más de 1.000 plataformas de imágenes (Reddit, Danbooru, ArtStation, Twitter/X, Instagram, foros de fotografía).

#### 5.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/tools/`
- **Archivos clave:**
  - `crates/tools/src/gallerydl.rs`: Detección de versión, parsing de JSON de metadatos (`--dump-json`) y ejecución en carpetas de trabajo aisladas.

#### 5.3. Implementación en Segmenta
1. **Ampliación de `segmenta-daemon/src/media.rs`:**
   - Detectar binario `gallery-dl` en `$PATH` o en `~/.local/share/segmenta/bin/`.
   - Si la URL introducida no es de video (fallo en probe yt-dlp) o coincide con dominios de galerías conocidas, disparar `gallery-dl --dump-json` para listar los elementos y encolar las descargas dentro de un paquete unificado.

---

### Módulo 6: Seguridad de Red: Interfaz VPN Dedicada y Kill Switch

#### 6.1. Justificación y Alcance
Para descargas de torrents o tráfico privado, los usuarios necesitan garantizar que el tráfico viaje **únicamente** a través de su interfaz VPN (ej. `tun0`, `wg0`, `mullvad-wireguard`). Si la VPN cae, las descargas deben detenerse inmediatamente para no revelar la IP residencial.

#### 6.2. Dónde estudiarlo en MDM
- **Directorios:** `crates/tools/src/relay.rs` y `crates/engine/src/`
- **Archivos clave:**
  - `crates/engine/src/network.rs`: Configuración del socket subyacente de `reqwest` / `tokio` usando `SO_BINDTODEVICE` en Linux.
  - `crates/tools/src/relay.rs`: Proxy SOCKS5/HTTP interno en `127.0.0.1` que fuerza a `yt-dlp`, `gallery-dl` y `librqbit` a salir exclusivamente por la interfaz seleccionada.

#### 6.3. Implementación en Segmenta
1. **Configuración de Socket en `segmenta-core`:**
   En `crates/segmenta-core/src/engine.rs`, en la construcción del cliente `reqwest::ClientBuilder`:
   ```rust
   #[cfg(target_os = "linux")]
   if let Some(interface) = &options.bind_interface {
       // Requiere CAP_NET_RAW o socket configurado a través de socket2 con SO_BINDTODEVICE
       use std::os::unix::io::AsRawFd;
       builder = builder.socket_factory(move |addr| {
           // Enlazar socket al dispositivo especificado antes del connect()
       });
   }
   ```
2. **Monitor de Kill Switch en Daemon:** Monitorear eventos de red de `rtnetlink` o sondeo periódico de interfaces activas (`getifaddrs`). Si la interfaz `tun0` desaparece, pausar inmediatamente todas las tareas encoladas con código de evento `NetworkInterfaceLost`.

---

### Módulo 7: Suplantación TLS / Anti-Bot Fingerprint Spoofing

#### 7.1. Justificación y Alcance
Muchos servicios de alojamiento y CDNs bloquean las firmas TLS por defecto de Rust (`rustls` o OpenSSL genérico) mediante análisis JA3/JA4. La emulación de huellas TLS de navegadores reales (Google Chrome) elimina errores 403 Forbidden artificiales.

#### 7.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/engine/`
- **Archivos clave:**
  - Integración de BoringSSL (`boring` / `tokio-boring` en Rust) configurando el conjunto de extensiones TLS, cipher suites y curvas elípticas idénticas a Chromium.

#### 7.3. Implementación en Segmenta
1. Evaluar la inclusión de `reqwest-impersonate` o `tokio-boring` como feature opcional (`--features impersonate`) en `segmenta-core` para no penalizar el tiempo de compilación por defecto.
2. Añadir en `ProbeOptions` un flag `impersonate_chrome: bool` que se active automáticamente cuando un servidor responda con Cloudflare Challenge o bloqueo de User-Agent.

---

### Módulo 8: Torrent Secuencial y Streaming en Descarga

#### 8.1. Justificación y Alcance
Permite al usuario comenzar a reproducir un archivo de audio o video mientras el torrent todavía se está descargando, priorizando los bloques iniciales y finales del archivo.

#### 8.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/torrent/`
- **Archivos clave:**
  - `crates/torrent/src/session.rs`: Uso de las APIs de `librqbit` para marcar un torrent con `sequential: true` y priorizar la pieza del cursor de reproducción.

#### 8.3. Implementación en Segmenta
1. En `crates/segmenta-daemon/src/torrent.rs`, exponer el método RPC `torrent.set_sequential(task_id, bool)`.
2. Habilitar en `segmenta-gui` una acción de menú contextual sobre descargas de video de torrent: **"Reproducir ahora (Secuencial)"**, que configure el modo secuencial y abra el archivo temporal con el reproductor multimedia predeterminado de GNOME (Totem, VLC, Celluloid).

---

### Módulo 9: Canales RSS para Torrents y Descargas Programadas

#### 9.1. Justificación y Alcance
Automatiza la descarga de podcasts, releases periódicas o capítulos de series desde feeds RSS con reglas de filtrado de texto (expresiones regulares o palabras clave).

#### 9.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/app/src/`
- **Archivos clave:**
  - `crates/app/src/rss.rs`: Parser RSS/Atom (`feed-rs`), persistencia de URLs ya vistas (`seen_guid`), temporizador de sondeo periódico.

#### 9.3. Implementación en Segmenta
1. **Esquema SQLite:**
   ```sql
   CREATE TABLE rss_feeds (
       id TEXT PRIMARY KEY,
       url TEXT NOT NULL,
       name TEXT NOT NULL,
       filter_regex TEXT,
       destination TEXT NOT NULL,
       check_interval_mins INTEGER DEFAULT 60,
       last_checked_at INTEGER
   );
   CREATE TABLE rss_seen_items (
       feed_id TEXT,
       guid TEXT,
       PRIMARY KEY (feed_id, guid)
   );
   ```
2. **Servicio en Daemon:** En `crates/segmenta-daemon/src/scheduler.rs`, añadir un bucle `tokio::time::interval` que evalúe los feeds activos y cree tareas automáticamente cuando encuentre nuevos items que coincidan con la expresión regular.

---

### Módulo 10: Extensiones Web: Popup de Descargas Recientes

#### 10.1. Justificación y Alcance
Evita que el usuario tenga que alternar continuamente entre el navegador y la ventana de Segmenta para abrir un archivo recién descargado.

#### 10.2. Dónde estudiarlo en MDM
- **Directorios:** `extension/` y `crates/nmhost/`
- **Archivos clave:**
  - `extension/popup.html` y `extension/popup.js`: Vista compacta con lista de descargas recientes, barra de búsqueda y estado en vivo.
  - `crates/nmhost/src/`: Protocolo de consulta `RecentAsk` y respuesta `RecentAnswer` mediante el canal Native Messaging.

#### 10.3. Implementación en Segmenta
1. **Extender `crates/segmenta-host/src/proxy.rs`:** Soportar un nuevo tipo de mensaje JSON:
   `{"type": "recent_tasks", "limit": 10}` que consulte al socket del daemon y responda de inmediato al popup del navegador.
2. **Interfaz de la Extensión:** En `extension/chromium/popup.html` y `extension/firefox/popup.html`, reemplazar el menú estático actual por una lista reactiva de descargas recientes con acciones de un clic ("Abrir carpeta", "Copiar link", "Reintentar").

---

### Módulo 11: Control Remoto Web y Modo Headless

#### 11.1. Justificación y Alcance
Permite controlar Segmenta instalado en un servidor doméstico (Home Server, NAS, Raspberry Pi) o gestionar las descargas desde el teléfono celular escaneando un código QR local sin necesidad de monitor.

#### 11.2. Dónde estudiarlo en MDM
- **Directorios:** `crates/remote/` y `crates/app/src/headless.rs`
- **Archivos clave:**
  - `crates/remote/src/server.rs`: Servidor HTTPS local basado en certificados autofirmados, tokens efímeros de emparejamiento y páginas WebAssembly o HTML estático empotrado.
  - `crates/remote/web/`: Interfaz web móvil minimalista.

#### 11.3. Implementación en Segmenta
1. Añadir flag al daemon: `segmenta-daemon --remote-control --port 8443`.
2. Generar en `segmenta-gui` o en el log del CLI un código QR (ASCII en terminal o imagen en GUI) que contenga la URL local y el token de acceso seguro (`https://192.168.1.X:8443/#token=...`).

---

### Módulo 12: Inhibición de Suspensión del Sistema (`Inhibit ScreenSaver`)

#### 12.1. Justificación y Alcance
Evita que el sistema operativo entre en modo reposo o desconecte la tarjeta de red en medio de una descarga de gran volumen.

#### 12.2. Dónde estudiarlo en MDM
- **Directorio:** `crates/app/src/unix.rs`
- **Archivos clave:**
  - Implementación del protocolo D-Bus `org.freedesktop.ScreenSaver` y `org.freedesktop.PowerManagement.Inhibit`.

#### 12.3. Implementación en Segmenta
1. En `crates/segmenta-daemon/src/scheduler.rs`:
   - Conectarse al bus del sistema D-Bus (`zbus`).
   - Al pasar de 0 tareas activas a 1 o más tareas activas: solicitar una cookie de inhibición con motivo: `"Segmenta tiene descargas activas en curso"`.
   - Cuando la cola se vacía o se pausa: liberar la cookie D-Bus para permitir el ahorro de energía normal del sistema.

---

## 3. Plan de Adopción por Releases

Para mantener la disciplina de desarrollo de Segmenta (OpenSpec, pruebas rigurosas y arquitectura desacoplada), se recomienda dividir estas mejoras en tres fases de versión:

### Fase 1: Release v0.4.0 (Automatización, Extracción y Asistentes)
- **Módulo 1:** Crate `segmenta-mcp` oficial para asistentes de IA.
- **Módulo 4:** Descompresión automática post-descarga (ZIP / TAR / 7Z).
- **Módulo 12:** Inhibición D-Bus de suspensión durante descargas.
- **Módulo 8:** Descarga secuencial en torrents para previsualización multimedia.

### Fase 2: Release v0.5.0 (Ecosistema Web, Debrid y Paquetes)
- **Módulo 2:** Módulo `debrid` en `segmenta-core` (Real-Debrid, AllDebrid, TorBox).
- **Módulo 3:** Servidor Click'n'Load en puerto 9666 y agrupación de tareas en paquetes.
- **Módulo 10:** Popup de descargas recientes en las extensiones de navegador.
- **Módulo 5:** Integración de `gallery-dl` para álbumes de imágenes.

### Fase 3: Release v0.6.0 (Red Avanzada y Control Remoto)
- **Módulo 6:** Atado a interfaz VPN (`SO_BINDTODEVICE`) con Kill Switch automático.
- **Módulo 7:** Suplantación TLS / bypass de bloqueos anti-bot con BoringSSL.
- **Módulo 11:** Servidor de control remoto Web con emparejamiento QR para móviles.
- **Módulo 9:** Canales RSS para torrents y descargas periódicas.

---

## 4. Consideraciones Legales y de Licencia

- El proyecto **Mister Download Manager** está publicado bajo la licencia **GNU General Public License v3.0 (GPL-3.0)**.
- **Segmenta** debe asegurar compatibilidad de licencias al estudiar, adaptar o incorporar código directo de MDM, manteniendo la atribución correspondiente en los encabezados de archivos fuente y en el archivo `ATTRIBUTION.md` del repositorio conforme a los términos de la GPL-3.0.
