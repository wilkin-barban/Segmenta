# OpenSpec 05: Guía de Implementación Paso a Paso para Agentes

> **Especificación:** OPENSPEC-SEGMENTA-005  
> **Versión:** 1.1.0  
> **Destinatarios:** Agentes de IA (Coders/Architects) y Desarrolladores

> **Contratos normativos:** 06 (HTTP), 07 (IPC), 08 (persistencia/hot-split), 09
> (extensión/Native Messaging) y 10 (empaquetado/pruebas). El roadmap no repite esos
> detalles: los referencia para que una decisión no viva en dos lugares.

> **Estado de las fases:** las fases 1–5 describen el camino recorrido hasta la GUI/daemon de
> **0.1.2**. Las fases **6–12** son el trabajo de **0.2.0** y están **(0.2, en preparación)**: no se
> leen como entregadas hasta que su fila del ODD esté en `done` con su gate verde. La única puerta
> al tag `v0.2.0` es la fase 12.

---

## 1. Decisión del Stack Tecnológico

Para garantizar que Segmenta sea ultrarrápido, consuma menos de 30 MB de RAM y se integre sin fisuras en Ubuntu 26.04, la arquitectura tecnológica seleccionada es:

* **Lenguaje Principal:** **Rust** (edición 2021/2024).
  * *Justificación:* Gestión de memoria sin pausas de Garbage Collector (esencial cuando 32 conexiones transfieren a 1 Gbps), concurrencia asíncrona de alto rendimiento con `tokio`, acceso directo y seguro a llamadas POSIX del kernel (`libc`), y bindings oficiales de GNOME de primer nivel (`gtk4-rs`, `libadwaita-rs`).
* **Capa de Red:** `reqwest` / `hyper` + `rustls` (o `native-tls`/OpenSSL).
* **Capa de E/S de Disco:** `libc::posix_fallocate` + `libc::pwrite64`.
* **Capa de Persistencia:** `rusqlite` con modo WAL.
* **Capa de GUI:** GTK4 + Libadwaita en Rust.
* **Extensión de Navegador:** JavaScript moderno vanilla (Manifest V3) sin dependencias pesadas.

---

## 2. Estructura de Workspace en Cargo

Cualquier agente que inicie el desarrollo debe estructurar el repositorio de código como un **Cargo Workspace**:

```
Segmenta/
├── Cargo.toml                  # Workspace manifest
├── crates/
│   ├── segmenta-core/          # Motor puro de descarga y segmentación (sin GUI)
│   ├── segmenta-daemon/        # Servicio IPC en segundo plano (Unix Socket)
│   ├── segmenta-host/          # Binario ELF para Native Messaging con Chrome/Firefox
│   ├── segmenta-cli/           # Herramienta de terminal para el usuario
│   └── segmenta-gui/           # Interfaz gráfica GTK4 + Libadwaita
├── extension/                  # Código fuente de la extensión WebExtension MV3
│   ├── manifest.json
│   ├── background.js
│   ├── content.js
│   ├── document.js
│   └── panel/
└── docs/                       # Especificaciones OpenSpec y Dossier Forense
```

---

## 3. Plan de Acción Detallado por Fases

### Fase 1: El Núcleo de Descargas (`segmenta-core`)

#### Tarea 1.1: Módulo de Sondeo (`probe.rs`)
* **Entrada:** `url: &str`, `headers: HeaderMap`, `cookies: &[Cookie]`.
* **Lógica:**
  1. Enviar una petición `reqwest::Client` con cabecera `Range: bytes=0-1`.
  2. Verificar si el código de estado es `StatusCode::PARTIAL_CONTENT` (206).
  3. Extraer `total_size` de la cabecera `Content-Range`.
  4. Extraer y sanitizar el nombre de archivo según `06-http-contract.md`.
  5. Guardar `ETag`/`Last-Modified`, `content_type` y `final_url` para la reanudación.
* **Salida:** `ProbeResult { total_size: Option<u64>, supports_ranges: bool, filename: String, etag: Option<String>, last_modified: Option<String> }`.

#### Tarea 1.2: Módulo de E/S de Disco (`disk.rs`)
* **Lógica:**
  1. Implementar la función `allocate_file(path: &Path, size: u64) -> Result<File>`:
     * Abrir con `OpenOptions::new().read(true).write(true).create(true).truncate(true)`.
     * Invocar `libc::posix_fallocate(file.as_raw_fd(), 0, size as libc::off_t)`.
     * Si retorna `ENOSPC`, propagar error de disco lleno.
     * Si retorna `EOPNOTSUPP`/`ENOSYS`, usar `ftruncate`/`set_len` y validar espacio con `fstatvfs`.
  2. Implementar `pwrite_all(file: &File, offset: u64, data: &[u8]) -> Result<()>`:
     * Repetir `FileExt::write_at` hasta vaciar el buffer, manejando `EINTR` y escrituras parciales (08).
     * No actualizar `next_offset` en SQLite hasta que el checkpoint de datos haya hecho `fdatasync`.

#### Tarea 1.3: Planificador de Segmentos en Caliente (`scheduler.rs`)
* **Estructura de Datos:**
  ```rust
  pub struct Segment {
      pub id: usize,
      pub start_offset: u64,
      pub end_offset: u64,
      pub next_offset: Arc<AtomicU64>,
      pub status: SegmentStatus,
  }
  ```
* **Lógica de Hot-Split:** implementar el protocolo cooperativo de 08, no cambiar `end_offset` desde el planificador sin confirmación:
    1. El worker ocioso notifica al planificador.
    2. El planificador elige el donante con mayor `(end_offset - next_offset) + 1`.
    3. Si el resto es `>= 2 MB`, envía `SplitRequest { target_midpoint }` al donante.
    4. El donante sólo confirma con `SplitAck` después de recortar su buffer y fijar el clamp en `target_midpoint`; nunca escribe bytes posteriores.
    5. Con la confirmación, el planificador persiste el nuevo segmento y lanza el worker en `[midpoint + 1, original_end]`.
    6. Sin confirmación (`SplitReject`), elige otro donante o deja al worker ocioso.

---

### Fase 2: Demonio e IPC (`segmenta-daemon` & `segmenta-host`)

#### Tarea 2.1: Servidor Unix Domain Socket (`ipc.rs`)
* Enlazar en `tokio::net::UnixListener::bind(&socket_path)`.
* Procesar paquetes NDJSON según [`07-ipc-contract.md`](07-ipc-contract.md), no sólo el resumen de 01.
* Mantener un canal de difusión (`tokio::sync::broadcast`) para notificaciones de progreso hacia los clientes conectados.

#### Tarea 2.2: Binario Native Messaging (`segmenta-host`)
* En `crates/segmenta-host/src/main.rs`:
  ```rust
  fn main() -> Result<(), Box<dyn Error>> {
      let mut stdin = std::io::stdin().lock();
      let mut stdout = std::io::stdout().lock();
      
      let mut length_buf = [0u8; 4];
      stdin.read_exact(&mut length_buf)?;
      let length = u32::from_le_bytes(length_buf) as usize; // little-endian, no `ne`
      
      let mut payload = vec![0u8; length];
      stdin.read_exact(&mut payload)?;
      
      // Enviar payload al Unix Domain Socket de segmenta-daemon
      // ...
  }
  ```

---

### Fase 3: Extensión de Navegador (`extension/`)

#### Tarea 3.1: Intercepción de Solicitudes y DOM
* Implementar `document.js` para capturar llamadas a `fetch` y URLs multimedia.
* Implementar `content.js` para inyectar el botón flotante sobre reproductores `<video>`.
* Implementar `background.js`:
  * Escuchar `chrome.runtime.onMessage`.
  * Extraer cookies con `chrome.cookies.getAll({ url })`.
  * Enviar mensaje a `segmenta-host` mediante `chrome.runtime.connectNative` o `sendNativeMessage` según el ciclo de vida definido en 09; validar gesto de usuario y esquema de URL.

---

### Fase 4: Interfaz de Usuario (`segmenta-gui`)

#### Tarea 4.1: Estructura Libadwaita
* Configurar `adw::Application` con ID `io.github.wilkin.Segmenta`.
* Crear `MainWindow` basada en `adw::NavigationSplitView`.
* Conectar con el socket del daemon en un hilo asíncrono y actualizar los widgets vía canales de GTK (`glib::MainContext::channel`).

#### Tarea 4.2: Implementación de `SegmentaSegmentView`
* Crear un widget personalizado derivado de `gtk::Widget`.
* En la señal `snapshot`, recorrer la lista de segmentos activos de la descarga seleccionada y renderizar los rectángulos coloreados con `gtk::Snapshot::append_color`.

---

### Fase 5: Manejo de Video Streaming (HLS / DASH)

#### Tarea 5.1: Parser de Manifiestos
* M3U8: Usar el crate `m3u8-rs` para extraer las resoluciones disponibles y fragmentos `.ts`/`.m4s`.
* MPD: Parsear con `quick-xml` para extraer la línea de tiempo y las URLs de inicialización de audio y video.

#### Tarea 5.2: Fusión con FFmpeg
* Seguir el pipeline de 09 §5: descargar el init segment y los fragmentos en orden, generar listas
  `ffconcat` separadas para video y audio y ejecutar FFmpeg con `-f concat -safe 0`, sin shell.
* **Alcance v1:** HLS/DASH públicos **sin cifrar**. Cualquier `EXT-X-KEY` con método distinto de
  `NONE` o `ContentProtection` DRM se rechaza; el comando simplista `-i video.m4s -i audio.m4s`
  no es suficiente para fragmentos reales.

---

### Fase 6: Confiabilidad y hardening (**R1–R3**) — (0.2, en preparación)

* **Contrato:** 03 §5.4 (cold-start de captura), 07 (probe de torrents y ayuda CLI), 10 (pruebas).
* **Estado:** pendiente en `odd/tasks/segmenta-0.2.md`; nada de esta fase está entregado.

#### Tarea 6.1: Cold-start de captura (R1)
* Archivos: `extension/chromium/background.js` y `extension/firefox/background.js` (copias
  sincronizadas por `extension/tools/sync-shared.mjs`).
* Al arrancar el worker (`runtime.onStartup`/`onInstalled` y un tick del watchdog), reconciliar con
  `downloads.search({ state: 'in_progress' })` reutilizando `decideCapture`, `matchExtension` y la
  denylist: no re-capturar ids ya `handled` ni capturas pendientes, y no pausar nada si la captura
  no está armada.
* Documentar el límite irreducible: si el navegador entregó la descarga antes de que existiera un
  worker, la reconciliación reduce la ventana pero no la elimina.
* Prueba (gate de la tarea): `node extension/tests/run.mjs`, con `downloads.search` falso y worker
  reiniciado en `watchdog.test.mjs`/`capture.test.mjs`.

#### Tarea 6.2: Probe multiarchivo de torrents (R2)
* Archivo: `crates/segmenta-daemon/src/torrent.rs`.
* Leer `meta.info.data.files` cuando exista: `total_bytes` como suma, `files[]` con
  índice/path/length/selected y `name`; mantener la ruta single-file para `.torrent` sin `files`.
* Prueba: fixtures bencode multiarchivo y single-file en `crates/segmenta-daemon/tests/`, con paths
  sanitizados. Actualizar 07 y 03 en el mismo cambio.

#### Tarea 6.3: `extension` en la ayuda CLI (R3)
* Archivo: `crates/segmenta-cli/src/cli.rs`.
* Añadir `extension` al texto de `--help` en es/en/pt-BR y revisar los flags de `media`, `torrent`,
  `settings` y `extension`.
* Prueba: `crates/segmenta-cli/tests/cli_help.rs`, que falla si un subcomando registrado no aparece
  en la ayuda.

* **Gate de la fase:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, `node extension/tests/run.mjs`,
  `node extension/tools/sync-shared.mjs --check`.

---

### Fase 7: Scheduler de colas y `max_active` (**S1–S2**) — (0.2, en preparación)

* **Contrato:** 12 (nuevo contrato de scheduler), 07 (`queue.schedule`, `max_active`,
  `action_on_complete`), 08 (dónde se persisten las ventanas), 11 (decisión y deuda).
* **Estado:** pendiente; hoy `max_active` se valida (1–8) y se guarda en la tabla `queues`, pero
  el daemon no lo aplica al arrancar tareas y no existe ningún scheduler de ventanas.

#### Tarea 7.1: `QueueScheduler` nativo (S1)
* Archivos: nuevo `crates/segmenta-daemon/src/scheduler.rs`, más `manager.rs`, `store.rs` y
  `main.rs`.
* Tarea `tokio` iniciada en `serve()`, tick de 30 s (pausable con `tokio::time::pause()` en tests).
* Fuente de verdad: `queue_schedule:<name>` en settings y los campos legacy
  `schedule_start`/`schedule_stop` para `main`; la ventana se evalúa en hora local (`chrono` con
  `clock`, nueva dependencia justificada).
* Centralizar el arranque en `start_pending_for_queue`; no sostener el mutex `running` durante
  `await`; despertar al scheduler al completar, fallar o cancelar una tarea.
* `action_on_complete` (`none|suspend|power_off|exit`) se ejecuta **sólo** cuando la cola queda
  vacía; `exit` usa el `shutdown` existente. Suspender/apagar exige permiso del sistema y se
  documenta como no verificable de forma destructiva en CI.
* Pruebas: ventana abre/cierra con reloj pausado; `queue.schedule` inválido devuelve error; no
  arranca tareas con el daemon inactivo; la acción se ejecuta una sola vez.

#### Tarea 7.2: `max_active` real por cola (S2)
* Archivos: `crates/segmenta-daemon/src/manager.rs` y sus tests.
* Aplicar `max_active` **antes** de llamar a `start_task` y liberar el slot al terminar, fallar o
  cancelar. La GUI no duplica este estado: sólo lee `queue.*`.
* Pruebas: nunca se supera `max_active`; una tarea terminada libera el slot y arranca la siguiente.

* **Gate de la fase:** `cargo test --workspace` (con los tests nuevos del scheduler y de
  `max_active`), más `cargo fmt`/`clippy`. La verificación real de una ventana corta y un
  `max_active` acotado es de Q1; nunca se prueba suspendiendo o apagando la máquina.

---

### Fase 8: Post-proceso y ClamAV (**A1–A2**) — (0.2, en preparación)

* **Contrato:** 12 (hook argv, ClamAV y cuarentena), 08 (migración a schema **v4**), 07 (settings y
  código `1046`), 04 §7.3 (estados en la UI), 10 (pruebas con motor falso).
* **Estado:** pendiente; hoy `postprocess_command` se persiste y el daemon no lo ejecuta.

#### Tarea 8.1: Hook argv seguro (A1)
* Archivos: nuevo `crates/segmenta-daemon/src/postprocess.rs`, más `manager.rs` y `store.rs`.
* Mantener `postprocess_command: Vec<String>` como argv: sustituir **sólo** `%file%`, `%folder%` y
  `%name%`; nunca shell; timeout configurable; stdout/stderr acotados.
* Un código de salida distinto de 0 no convierte una descarga correcta en fallida salvo que se
  configure explícitamente; se registra en `postprocess_exit`/`postprocess_message`.
* Pruebas: `%file%` con espacios en la ruta, ausencia de shell, timeout, salida no cero.

#### Tarea 8.2: ClamAV integrado y cuarentena (A2)
* Archivos: `postprocess.rs`, `manager.rs`, `store.rs`, `rpc.rs`, `schema.sql` (migración v4) y la
  UI de la fase 10.
* Settings nuevos: `antivirus_enabled` (false), `antivirus_timeout_secs` (120),
  `antivirus_quarantine_dir` (`$XDG_DATA_HOME/segmenta/quarantine`).
* Detectar `clamdscan` y luego `clamscan`. Con `antivirus_enabled` y sin motor: tarea `failed` con
  `ERR_ANTIVIRUS_UNAVAILABLE`; nunca se publica `completed`.
* Flujo: `finalizing` → `verifying` → escaneo/hook → `completed` o cuarentena.
* Infección: mover el archivo a cuarentena, marcar `failed` con `ERR_ANTIVIRUS_FOUND` (código
  `1046`), guardar `quarantine_path` y el resultado; nunca dejar el archivo en Descargas.
* Migración v4: columnas `quarantine_path`, `postprocess_exit`, `postprocess_message`; actualizar
  `DownloadRecord`, `DownloadView` y el modelo de la GUI.
* Pruebas: `clamscan`/`clamdscan` falsos (limpio, infectado, ausente, timeout), cuarentena sin
  archivo en Descargas y tarea nunca `completed`.

* **Gate de la fase:** `cargo test --workspace` y `xvfb-run -a cargo test -p segmenta-gui` (sin
  skips por falta de display), más `cargo fmt`/`clippy`.

---

### Fase 9: Site logins Basic + keyring (**L1**) — (0.2, en preparación)

* **Contrato:** 12 (auth), 07 (`site_login.list|set|remove`, headers sensibles), 08 (tabla
  `site_logins` y alias de keyring), 09 (límites de sitio), 04 §6 (página de Preferencias).
* **Estado:** pendiente; el esquema v3 ya declara la tabla `site_logins`
  (`domain`/`username`/`keyring_secret_id`) pero ningún RPC ni código del daemon la usa todavía.

#### Tarea 9.1: Almacenamiento y RPC (L1)
* Archivos: `store.rs`, `manager.rs`, `rpc.rs`, `crates/segmenta-gui/src/prefs.rs` y un test nuevo
  (`crates/segmenta-daemon/tests/site_logins.rs` o `secret_backend.rs`).
* `username` en SQLite y contraseña en el keyring con alias `site/<domain>`; **nunca** se devuelve
  `password`. Sin keyring: error `ERR_KEYRING_UNAVAILABLE` y no se crea la entrada.
* Validar dominio exacto (sin comodines) y URL http/https sin credenciales embebidas.
* Al añadir, iniciar o probar una URL, inyectar `Authorization: Basic` como cabecera sensible; una
  cabecera explícita de la tarea gana y se mantiene la regla de no reenviar credenciales en
  redirects cross-origin.
* Un 401 sin login sigue siendo `ERR_AUTH_REQUIRED`: no hay fallback silencioso a Basic.
* Sin Digest; se documenta como limitación, no como promesa.
* Pruebas: Basic correcto contra servidor local, contraseña incorrecta, sin keyring, secreto
  ausente de logs y de `settings.get`, y no envío en redirect externo.

* **Gate de la fase:** `cargo test --workspace` (incluye el test de site logins) más
  `cargo fmt`/`clippy`; el secreto no debe aparecer en la salida de ningún test.

---

### Fase 10: UI de 0.2 (**U1**) — (0.2, en preparación)

* **Contrato:** 04 §6, §7.3 y §8 (páginas, estados y deuda a cerrar), 07 (nombres exactos de RPC y
  settings), 12 (semántica de scheduler y cuarentena).
* **Estado:** pendiente; hoy no existen las páginas de site logins ni de antivirus y las categorías
  siguen pintadas en sólo lectura.

#### Tarea 10.1: Páginas y estados nuevos (U1)
* Archivos: `crates/segmenta-gui/src/prefs.rs`, `window.rs`, `model.rs`, `add_dialog.rs`.
* Página **Contraseñas de sitios** con añadir/editar/borrar por dominio y diálogo de contraseña
  aparte; la contraseña nunca se muestra ni llega a `settings.get`.
* Sección **Antivirus** en Preferencias (interruptor, motor detectado, timeout, carpeta de
  cuarentena) y estado «En cuarentena» con `quarantine_path` en el detalle.
* Retirar el `AdwExpanderRow` de categorías; `category_dirs` se sigue leyendo para bases antiguas,
  sin UI y sin clasificación automática.
* Resolver `clip_monitor_enabled` con una sola semántica documentada (respetarlo o retirarlo del
  modelo) y dar etiqueta y acciones a `retrying`/`finalizing` (04 §7.2).
* Mover a `segmenta-i18n` las cadenas incrustadas listadas en 04 §8, incluidas las de
  notificación, reutilizando las claves que ya existen.
* Pruebas: `xvfb-run -a cargo test -p segmenta-gui`, con cobertura de que el secreto no aparece en
  la UI y de que la lista de acciones del detalle respeta el estado.

* **Gate de la fase:** `xvfb-run -a cargo test -p segmenta-gui` sin skips por display, más
  `cargo fmt`/`clippy` y `cargo test --workspace`.

---

### Fase 11: Extensión manual y límites de alcance (**E1**) — (0.2, en preparación)

* **Contrato:** 09 (permisos reales, ID real, instalación manual), 03 (sniffer YouTube-only y
  captura genérica), 04 §6 (página Navegadores), 10 (ZIP de extensión).
* **Estado:** pendiente; la instalación asistida ya existe en la GUI, pero sus textos y límites no
  están alineados con este documento.

#### Tarea 11.1: Instalación asistida y límites (E1)
* Archivos: `crates/segmenta-daemon/src/browsers.rs`, `crates/segmenta-gui/src/prefs.rs`, docs.
* Mejorar textos y estados de `extension.prepare`/`extension.open` para la carga manual en Chrome
  for Testing y Firefox oficial; sin Chrome Web Store ni AMO, sin firma ni política.
* Mantener el sniffer sólo para YouTube más la captura genérica existente; no prometer Vimeo ni
  Twitter.
* Documentar la limitación de Firefox snap (el host nativo no puede ejecutarse) sin intentar burlar
  el sandbox, y el handler de magnet con `xdg-mime`; si el empaquetado lo permite, registrarlo sin
  acción manual.
* Pruebas: `node extension/tests/run.mjs`, `node extension/tools/sync-shared.mjs --check`,
  `node extension/tools/pack.mjs --check`.

* **Gate de la fase:** los tres comandos Node del punto anterior más `cargo test --workspace`.

---

### Fase 12: QA, empaquetado y release (**Q1**) — (0.2, en preparación)

* **Contrato:** 10 (gates y `.deb`/ZIP 0.2.0), `docs/qa/segmenta-0.2-live.md`,
  `packaging/release-notes-0.2.0.md`, `README`/`CHANGELOG` y `odd/tasks/segmenta-0.2.md`.
* **Estado:** pendiente; es la única puerta al tag `v0.2.0`, y hasta que esté verde la release se
  describe como **(0.2, en preparación)**.

#### Tarea 12.1: Gates completos
```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
xvfb-run -a cargo test -p segmenta-gui
node extension/tests/run.mjs
node extension/tools/sync-shared.mjs --check
node extension/tools/pack.mjs --check
packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0
```
* Ningún skip silencioso por display o dependencia cuenta como verde (Definition of Done del ODD).

#### Tarea 12.2: QA real en Ubuntu
* Instalar el `.deb` 0.2.0 y arrancar por systemd; `extension.status`/`prepare`/`open` en Chrome for
  Testing y Firefox oficial.
* Panel YouTube 4K/1080p/720p/360p + audio con 720p por defecto; captura `.zip` con confirmación
  (aceptar, rechazar, timeout, host caído) y cold-start tras arrancar Chrome.
* Scheduler con ventana corta y `max_active` sobre fixtures locales; ClamAV con fixture EICAR,
  cuarentena y tarea fallida sin archivo en Descargas; site login Basic contra servidor local;
  torrent multiarchivo; `segmenta --help` con `extension`.
* Limpiar procesos, audio, fixtures, tareas y dejar `allow_local_downloads=false`.

#### Tarea 12.3: Release
* Actualizar `README`, `CHANGELOG`, `docs/qa/segmenta-0.2-live.md` y el ODD con SHA-256 reales.
* Tag `v0.2.0` y GitHub Release con `.deb`, `.sha256`, ZIP de Chromium, ZIP de Firefox, notas y
  docs. Ningún gate rojo publica.

* **Gate de la fase:** los ocho comandos de 12.1 verdes, la matriz de QA de 12.2 completa y sin
  pendientes, y `git status` limpio antes de etiquetar.

---

## 4. Criterios de Aceptación y Pruebas Requeridas

1. **Test de Concurrencia y Rangos:**
   * Descargar un archivo de prueba de 1 GB desde un servidor Nginx con soporte de rangos.
   * Comprobar que se generan exactamente $N$ conexiones simultáneas.
   * Verificar que la suma de bytes escritos coincide con el SHA-256 del archivo original.
2. **Test de División en Caliente (*Hot-Split*):**
   * Emular una conexión estrangulada a 10 KB/s mientras las demás van a 10 MB/s.
   * Validar que los workers rápidos subdividen el segmento lento repetidamente.
3. **Test de Reanudación de Descarga:**
   * Interrumpir abruptamente la conexión a la mitad de la descarga (`SIGKILL` sobre el proceso).
   * Relanzar el proceso y verificar que se reanuda desde los últimos desplazamientos atómicos sin reiniciar desde cero.
4. **Test de Rendimiento de Disco:**
   * Comparar tiempo de escritura final frente a gestores tradicionales basados en concatenación de temporales. Confirmar que en Segmenta el tiempo de ensamblado es **0.0 segundos**.
