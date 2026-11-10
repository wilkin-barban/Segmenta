# OpenSpec 05: Guía de Implementación Paso a Paso para Agentes

> **Especificación:** OPENSPEC-SEGMENTA-005  
> **Versión:** 1.0.0  
> **Destinatarios:** Agentes de IA (Coders/Architects) y Desarrolladores

---

## 1. Decisión del Stack Tecnológico

Para garantizar que Segmenta sea ultrarrápido, consuma menos de 30 MB de RAM y se integre sin fisuras en Ubuntu 26.10, la arquitectura tecnológica seleccionada es:

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
  4. Extraer el nombre de archivo de `Content-Disposition` o del último segmento de la URL.
* **Salida:** `ProbeResult { total_size: Option<u64>, supports_ranges: bool, filename: String }`.

#### Tarea 1.2: Módulo de E/S de Disco (`disk.rs`)
* **Lógica:**
  1. Implementar la función `allocate_file(path: &Path, size: u64) -> Result<File>`:
     * Abrir con `OpenOptions::new().read(true).write(true).create(true).truncate(true)`.
     * Invocar `libc::posix_fallocate(file.as_raw_fd(), 0, size as libc::off_t)`.
     * Si retorna código de error `ENOSPC`, propagar error de disco lleno.
  2. Implementar `write_at(file: &File, offset: u64, data: &[u8]) -> Result<usize>`:
     * Invocar `libc::pwrite64(file.as_raw_fd(), data.as_ptr() as *const libc::c_void, data.len(), offset as libc::off_t)`.

#### Tarea 1.3: Planificador de Segmentos en Caliente (`scheduler.rs`)
* **Estructura de Datos:**
  ```rust
  pub struct Segment {
      pub id: usize,
      pub start_offset: u64,
      pub end_offset: u64,
      pub current_offset: Arc<AtomicU64>,
      pub status: SegmentStatus,
  }
  ```
* **Lógica de Hot-Split:**
  * Al recibir la notificación de que el worker $k$ finalizó su segmento:
    1. Bloquear el mutex del planificador.
    2. Filtrar segmentos activos donde `(end_offset - current_offset) > 2 * 1024 * 1024` (2 MB).
    3. Seleccionar el segmento con mayor delta restante.
    4. Calcular `midpoint = current_offset + (remaining / 2)`.
    5. Actualizar `end_offset` del segmento emisor a `midpoint`.
    6. Instanciar un nuevo `Segment` con rango `(midpoint + 1)..=original_end`.
    7. Lanzar una nueva tarea en `tokio::spawn` para el worker ocioso.

---

### Fase 2: Demonio e IPC (`segmenta-daemon` & `segmenta-host`)

#### Tarea 2.1: Servidor Unix Domain Socket (`ipc.rs`)
* Enlazar en `tokio::net::UnixListener::bind(&socket_path)`.
* Procesar paquetes NDJSON según el esquema documentado en `OpenSpec 01`.
* Mantener un canal de difusión (`tokio::sync::broadcast`) para notificaciones de progreso hacia los clientes conectados.

#### Tarea 2.2: Binario Native Messaging (`segmenta-host`)
* En `crates/segmenta-host/src/main.rs`:
  ```rust
  fn main() -> Result<(), Box<dyn Error>> {
      let mut stdin = std::io::stdin().lock();
      let mut stdout = std::io::stdout().lock();
      
      let mut length_buf = [0u8; 4];
      stdin.read_exact(&mut length_buf)?;
      let length = u32::from_ne_bytes(length_buf) as usize;
      
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
  * Enviar mensaje a `segmenta-host` mediante `chrome.runtime.sendNativeMessage("io.github.wilkin.segmenta", payload)`.

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
* Una vez descargados los segmentos en disco, invocar mediante `tokio::process::Command`:
  ```bash
  ffmpeg -y -i video.m4s -i audio.m4s -c copy -movflags +faststart salida.mp4
  ```

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
