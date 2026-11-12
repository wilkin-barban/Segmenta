# OpenSpec 07: Contrato IPC, Protocolo JSON-RPC y Control de Ciclo de Vida

> **Especificación:** OPENSPEC-SEGMENTA-007  
> **Versión:** 1.0.0  
> **Área:** Comunicación Inter-Procesos (IPC), Seguridad de Sockets Unix y Catálogo de Eventos

---

## 1. Transporte, Autenticación y Ciclo de Vida del Socket

### 1.1. Ubicación y Permisos del Socket
* **Ruta del Socket Unix:**  
  `$XDG_RUNTIME_DIR/segmenta/daemon.sock`  
  (Normalmente `/run/user/<UID>/segmenta/daemon.sock`).
* **Permisos del Directorio y Socket:**
  * El directorio contenedor `$XDG_RUNTIME_DIR/segmenta` DEBE crearse con permisos **`0700`** (`rwx------`).
  * El archivo de socket DEBE tener permisos **`0600`** (`rw-------`).

### 1.2. Autenticación de Pares (`SO_PEERCRED`)
Para prevenir que otros procesos de usuarios en la misma máquina interactúen con el daemon:
1. En cada conexión entrante aceptada por el socket Unix, el daemon ejecuta:
   ```c
   struct ucred cr;
   socklen_t len = sizeof(struct ucred);
   getsockopt(client_fd, SOL_SOCKET, SO_PEERCRED, &cr, &len);
   ```
2. **Validación:** El daemon verifica que `cr.uid == getuid()`.
3. Si el UID del cliente no coincide con el UID del propietario del daemon, la conexión se **cierra inmediatamente** sin enviar datos ni procesar mensajes.

---

### 1.3. Ciclo de Vida y Socket Activation con Systemd
* **Unidad Socket (`segmenta.socket`):**
  ```ini
  [Unit]
  Description=Segmenta Download Engine Activation Socket

  [Socket]
  ListenStream=%t/segmenta/daemon.sock
  SocketMode=0600
  DirectoryMode=0700

  [Install]
  WantedBy=sockets.target
  ```
* **Unidad Servicio (`segmenta.service`):**
  ```ini
  [Unit]
  Description=Segmenta Core Download Daemon
  Requires=segmenta.socket
  After=network.target

  [Service]
  Type=exec
  ExecStart=/usr/bin/segmenta-daemon --systemd
  Restart=on-failure
  RestartSec=2s
  StandardOutput=journal
  StandardError=journal
  ```
* **Instancia Única Standalone (sin systemd):** Si el daemon se ejecuta fuera de systemd, adquiere un bloqueo exclusivo mediante `flock(fd, LOCK_EX | LOCK_NB)` sobre `$XDG_RUNTIME_DIR/segmenta/daemon.lock`. Si el archivo ya está bloqueado, el proceso aborta reportando `ERR_DAEMON_ALREADY_RUNNING`.

---

### 1.4. Versión del protocolo
* La versión inicial es `v1`; el cliente envía `system.hello` al conectar y el daemon responde con
  `protocol_version: 1`, versión del binario y `min_client_version: 1`.
* Un cliente con versión mayor o menor incompatible recibe `-32600 INVALID_REQUEST` con
  `data.code = "ERR_PROTOCOL_VERSION"` y no se le permite enviar comandos.
* Las notificaciones no llevan `id`; las respuestas conservan el `id` del cliente. Un `id` repetido
  mientras la petición está en vuelo es `ERR_DUPLICATE_REQUEST_ID`.

## 2. Framing, Límites y Control de Flujo

1. **Framing:** JSON-RPC 2.0 delimitado por salto de línea (`\n`, formato NDJSON).
2. **Tamaño Máximo de Mensaje:** **1 MiB (1,048,576 bytes)**.
   * Si un cliente transmite más de 1 MiB sin un byte de salto de línea, el daemon aborta la conexión con error `1009 (ERR_PAYLOAD_TOO_LARGE)` para prevenir saturación de memoria.
3. **Manejo de Desconexiones:**
   * La desconexión de la GUI o de la extensión **NUNCA detiene las descargas activas**. Las descargas son propiedad del daemon y continúan en segundo plano.
4. **Buffer de Retransmisión y Backpressure:**
   * El daemon mantiene un canal de difusión (*broadcast channel*) con buffer circular de **1024 eventos**.
   * Si un cliente GUI se bloquea o procesa eventos más lento de lo que se emiten y pierde eventos, el daemon envía una notificación `{"method": "system.resync_required"}` y el cliente debe reiniciar su caché local invocando `download.list`.

---

## 3. Catálogo Exhaustivo de Métodos JSON-RPC

Todos los mensajes siguen la especificación JSON-RPC 2.0 y declaran `"jsonrpc": "2.0"`.

### 3.1. `download.add`
* **Parámetros:**
  * `url` (string, obligatorio): URL HTTP/HTTPS válida.
  * `destination_dir` (string, opcional): Ruta absoluta. Si se omite, se usa la categoría correspondiente.
  * `filename` (string, opcional): Nombre sugerido sanitizado.
  * `headers` (object, opcional): Mapa de cabeceras HTTP (`User-Agent`, `Referer`, etc.).
  * `cookies` (array de objetos, opcional): `[{"name": "...", "value": "...", "domain": "...", "path": "..."}]`.
  * `connections` (integer, opcional): 1 a 32 (default: 16).
  * `expected_checksum` (object, opcional): `{"algorithm": "sha256", "hash": "..."}`.
  * `start_immediately` (boolean, opcional, default: true).
  * `queue_name` (string, opcional, default: `"main"`).
  * `priority` (integer, opcional, default: 0; mayor = antes).
* **Respuesta (`result`):**
  * `task_id` (string UUIDv4).
  * `status` (string: `"pending"` | `"probing"` | `"downloading"`).
  * `filename` (string): nombre saneado de la tarea.
  * `destination_dir` (string): **carpeta efectiva** donde se escribe el archivo, es decir
    `destination_dir` del pedido o, si se omitió, el `download_dir` vigente.
  * `final_path` (string): ruta absoluta **resuelta y desduplicada** que producirá la tarea
    (`manual (1).deb` cuando `manual.deb` ya existe en la carpeta).
* La ventana de confirmación de captura (S10) muestra exactamente ese par, y los mismos dos
  campos viajan en el evento `task_added`. Ningún cliente necesita resolver la ruta por su
  cuenta.

### 3.2. `download.pause`
* **Parámetros:** `{"task_id": "c1f7a28e-..."}`
* **Comportamiento:** Cancela las tareas de red activas, vuelca el progreso a SQLite y cambia estado a `paused`.
* **Respuesta:** `{"result": {"task_id": "...", "status": "paused"}}`.

### 3.3. `download.resume`
* **Parámetros:** `{"task_id": "c1f7a28e-..."}`
* **Comportamiento:** Valida caché (`If-Range`), reactiva workers y reanuda desde los offsets guardados.
* **Respuesta:** `{"result": {"task_id": "...", "status": "downloading"}}`.

### 3.4. `download.cancel`
* **Parámetros:**
  * `task_id` (string, obligatorio).
  * `delete_file` (boolean, opcional, default: false).
* **Respuesta:** `{"result": {"task_id": "...", "status": "cancelled", "file_deleted": false}}`.

### 3.5. `download.delete`
* **Parámetros:**
  * `task_id` (string, obligatorio).
  * `delete_file` (boolean, obligatorio).
* **Comportamiento:** Elimina el registro de la base de datos y borra el archivo físico si se indicó.
* **Respuesta:** `{"result": {"task_id": "...", "deleted": true}}`.

### 3.6. `download.status`
* **Parámetros:** `{"task_id": "c1f7a28e-..."}`
* **Respuesta (`result`):**
  ```json
  {
    "task_id": "c1f7a28e-...",
    "url": "https://example.com/file.iso",
    "final_path": "/home/user/Descargas/file.iso",
    "status": "downloading",
    "total_bytes": 4294967296,
    "downloaded_bytes": 1073741824,
    "speed_bps": 52428800,
    "eta_seconds": 61,
    "retry_count": 0,
    "supports_ranges": true,
    "segments": [
      {
        "id": 1,
        "start_offset": 0,
        "end_offset": 2147483647,
        "next_offset": 1073741824,
        "status": "active",
        "speed_bps": 26214400
      }
    ]
  }
  ```

### 3.7. `download.list`
* **Parámetros:**
  * `filter` (string, opcional: `"all"` | `"active"` | `"paused"` | `"completed"` | `"failed"`).
  * `limit` (integer, opcional, default: 100, max: 1000).
  * `offset` (integer, opcional, default: 0).
* **Respuesta:** Array de objetos con, como mínimo:
  `task_id`, `url`, `final_path`, `status`, `total_bytes`, `downloaded_bytes`, `speed_bps`,
  `eta_seconds`, `retry_count`, `queue_name`, `created_at`, `completed_at`.
  El orden es `created_at DESC`; `limit`/`offset` se aplican después del filtro.

### 3.8. `settings.get` y `settings.update`
* **`settings.get`:** Retorna el diccionario completo de opciones de configuración.
* **`settings.update`:**
  * **Parámetros:** Diccionario parcial o completo con claves válidas (ej. `max_connections`, `global_speed_limit_bps`, `clip_monitor_enabled`, etc.). Valida tipos y rangos numéricos.

### 3.9. Catálogo de settings v1
`settings.update` acepta sólo estas claves; las desconocidas son `-32602`:

| Clave | Tipo | Default | Rango / nota |
| --- | --- | --- | --- |
| `max_connections` | integer | 16 | 1–32 |
| `global_speed_limit_bps` | integer/null | null | 0 = ilimitado; máximo 1 GiB/s |
| `clip_monitor_enabled` | boolean | true | |
| `autostart` | boolean | false | systemd --user |
| `download_dir` | string | `$XDG_DOWNLOAD_DIR` | ruta absoluta existente |
| `category_dirs` | object | vacío | categoría → ruta absoluta |
| `capture_extensions` | array[string] | lista de 04 ampliada en 0.1.2 | sin punto inicial; incluye `deb`, `rpm`, `appimage`, `xz`, `whl`, `jar`, `epub`, `vsix`, `xpi`, `msix`. Una fila escrita por 0.1.0/0.1.1 cuya lista sea byte a byte su default histórico se actualiza **por unión**: primero los defaults de 0.1.2 y luego los tokens que sólo tenía esa lista (`dmg`, `flac`, `tgz`, `webm` en 0.1.0), para no dejar de capturar formatos existentes; una lista personalizada no se toca y la actualización es idempotente |
| `capture_confirm` | boolean | true | pausa la descarga del navegador y abre la ventana de confirmación con nombre, host y carpeta efectiva; `false` captura sin preguntar |
| `domain_denylist` | array[string] | [] | host exacto o sufijo explícito |
| `schedule_start` | string/null | null | `HH:MM`, zona `America/Sao_Paulo` |
| `schedule_stop` | string/null | null | `HH:MM` |
| `action_on_complete` | string | `none` | `none/suspend/power_off/exit` |
| `allow_local_downloads` | boolean | false | habilita SSRF a LAN explícitamente |
| `max_retries` | integer | 5 | 0–20 |
| `proxy_mode` | string | `system` | `system/none/manual` |
| `proxy_url` | string/null | null | validada por esquema |
| `postprocess_command` | array[string] | [] | sin shell; `%file%` como token |

`settings.get` devuelve exactamente estas claves y sus valores efectivos. Los secretos de proxy y de
sitios no se devuelven: viven en el llavero (08) y se referencian por alias.

### 3.10. `system.hello`
* **Parámetros:** `{"client_name": "...", "client_version": "..."}`.
* **Respuesta:** `{"protocol_version": 1, "daemon_version": "...", "min_client_version": 1}`.

### 3.11. `queue.schedule`
* **Parámetros:**
  * `queue_name` (string): `"main"` o personalizada.
  * `start_time` (string nullable: `"HH:MM"`).
  * `stop_time` (string nullable: `"HH:MM"`).
  * `action_on_complete` (string: `"none"` | `"suspend"` | `"power_off"` | `"exit"`).

### 3.12. `queue.list`, `queue.create`, `queue.remove`
* **`queue.list`:** devuelve `[{"name": "main", "active": 2, "pending": 5, "paused": 1}]`.
* **`queue.create`:** `{"name": "noche", "max_active": 1}`; nombre único, 1–64 caracteres `[a-z0-9_-]`.
* **`queue.remove`:** `{"name": "noche"}`; sólo si no tiene tareas activas; las pendientes se mueven a `main`.
* La cola `main` no se puede eliminar ni renombrar.

### 3.13. `system.status` y `system.shutdown`
* **`system.status`:** Retorna versión del daemon, uptime en segundos, memoria en uso y conexiones activas.
* **`system.shutdown`:** Pausa limpiamente todas las tareas activas, ejecuta checkpoint WAL y finaliza el proceso daemon con código de salida 0.

### 3.14. `torrent.probe`, `torrent.add` y `torrent.status`
* **`torrent.probe`:** `{"url": "magnet:?xt=urn:btih:..."}` o una URL `http(s)` de `.torrent`.
  Devuelve `name`, `info_hash`, `total_bytes`, `files[]` y `source_type` (`magnet` | `torrent`).
* **`torrent.add`:**
  * **Parámetros:** `url` (obligatorio), `destination_dir` (string, opcional, ruta absoluta;
    si se omite se usa `download_dir`), `seed_after_complete` (boolean, opcional, default false).
  * **Respuesta:** `task_id`, `status` (`"downloading"`), `filename`, `info_hash`,
    `destination_dir` (carpeta efectiva) y `final_path` (ruta resuelta y desduplicada del
    payload). El evento `task_added` lleva el mismo par.
  * Las rutas internas del torrent se sanean contra `destination_dir`: rutas absolutas,
    `..` y cualquier escape de la carpeta base se rechazan con `-32602`.
* **`torrent.status`:** `{"task_id": "..."}`; devuelve el estado de `download.status` más
  `info_hash`, `seed_after_complete`, `uploaded_bytes`, `peers`, `files_count`, `files[]`,
  `final_path` y `destination_dir`.

---

## 4. Tabla de Códigos de Error

| Código | Identificador | Causa y Acción |
| :--- | :--- | :--- |
| **`-32700`** | `PARSE_ERROR` | JSON corrupto recibido por el socket. |
| **`-32600`** | `INVALID_REQUEST` | El payload no cumple con la estructura JSON-RPC 2.0. |
| **`-32601`** | `METHOD_NOT_FOUND` | Método no registrado en el dispatcher. |
| **`-32602`** | `INVALID_PARAMS` | Parámetros inválidos, tipos incorrectos o fuera de rango. |
| **`-32603`** | `INTERNAL_ERROR` | Fallo interno no controlado en el daemon. |
| **`1001`** | `TASK_NOT_FOUND` | El `task_id` proporcionado no existe en la base de datos. |
| **`1002`** | `TASK_ALREADY_ACTIVE` | Se intentó reanudar una descarga que ya está en ejecución. |
| **`1003`** | `TASK_NOT_RESUMABLE` | El servidor original no soporta `Range` y la descarga fue interrumpida. |
| **`1004`** | `INVALID_URL` | La URL no tiene esquema `http://` o `https://` o no resuelve por DNS. |
| **`1005`** | `DESTINATION_UNWRITABLE` | Permisos denegados en la carpeta destino (`EACCES`). |
| **`1006`** | `DISK_FULL` | `posix_fallocate` devolvió `ENOSPC`. |
| **`1007`** | `SSRF_BLOCKED_IP` | La URL resuelve a una IP de red privada o de bucle invertido. |
| **`1008`** | `CHECKSUM_MISMATCH` | El hash del archivo final no coincide con el digest esperado. |
| **`1009`** | `PAYLOAD_TOO_LARGE` | Mensaje entrante superior a 1 MiB. |
| **`1010`** | `UNAUTHORIZED_PEER` | El UID del proceso cliente difiere del UID del daemon. |

---

## 5. Catálogo de Notificaciones Push y Sincronización de Eventos

Todos los eventos emitidos por el daemon incluyen un identificador de secuencia monótono `seq_id` para permitir que los clientes detecten desincronización:

```json
{
  "jsonrpc": "2.0",
  "method": "download.event",
  "params": {
    "seq_id": 4821,
    "timestamp_ms": 1794356400000,
    "event_type": "state_changed",
    "task_id": "c1f7a28e-...",
    "data": {
      "old_state": "probing",
      "new_state": "downloading"
    }
  }
}
```

### Eventos Definidos:
1. **`task_added`**: Emitido cuando una nueva tarea ingresa a la cola. Lleva la ubicación efectiva de la tarea (`destination_dir` y `final_path`) para todas las vías de alta: `download.add`, `media.add`, manifiestos y `torrent.add`.
2. **`state_changed`**: Emitido en cualquier transición de la máquina de estados.
3. **`segment_started`**: Un worker inicia la descarga de un bloque de bytes.
4. **`segment_split`**: Un segmento activo fue dividido en caliente (contiene `donor_id`, `new_segment_id`, `midpoint`).
5. **`segment_completed`**: Un worker finalizó exitosamente su rango de bytes.
6. **`task_progress`**: Emitido a una tasa máxima regulada de **4 eventos por segundo** por descarga activa para no saturar el bus de eventos ni el renderizado de la GUI. Contiene bytes acumulados, velocidad instantánea en bytes/s y ETA.
7. **`task_completed`**: Descarga terminada y validada en disco.
8. **`task_failed`**: Error terminal en la descarga (incluye código y mensaje legible).

9. **`download_restarted_due_to_stale_cache`**: el servidor devolvió `200` a una reanudación con
   `If-Range` o cambió el `total` de `Content-Range`; el motor truncó el archivo y reinició los
   segmentos desde cero (lo exige `06`, sección 2.2). `data` lleva `reason`.

### Payload mínimo por evento

| `event_type` | `data` obligatorio |
| --- | --- |
| `task_added` | `task_id`, `status`, `url`, `destination_dir`, `final_path` |
| `state_changed` | `old_state`, `new_state`, `error_code?` |
| `segment_started` | `segment_id`, `start_offset`, `end_offset` |
| `segment_split` | `donor_segment_id`, `new_segment_id`, `midpoint`, `original_end` |
| `segment_completed` | `segment_id`, `start_offset`, `end_offset` |
| `task_progress` | `downloaded_bytes`, `total_bytes?`, `speed_bps`, `eta_seconds?`, `segments[]` |
| `task_completed` | `final_path`, `total_bytes`, `checksum_ok` |
| `task_failed` | `error_code`, `message`, `retryable` |

`seq_id` es monótono por daemon, no por cliente. Un cliente que detecta un salto debe llamar a
`download.list` y descartar su caché; no se intenta reenviar eventos perdidos.

---

## 9. Contrato de medios (0.1.0)

### 9.1. `media.probe`

```json
{"jsonrpc":"2.0","id":1,"method":"media.probe",
 "params":{"url":"https://www.youtube.com/watch?v=...","kind":"video","limit":100}}
```

Respuesta:

```json
{"result":{"kind":"video","title":"...","duration_seconds":213,"thumbnail":"https://...",
 "formats":[{"format_id":"22","label":"720p","height":720,"ext":"mp4","kind":"video"}],
 "audio":[{"format_id":"251","label":"webm","ext":"webm","kind":"audio"}],
 "qualities":[{"quality":"720","label":"720p","height":720,"format_id":"22"},
              {"quality":"audio","label":"audio","height":null,"format_id":"251"}],
 "default_quality":"720",
 "playlist":{"count":0,"entries":[]}}}
```

- `kind` ∈ `video|playlist|stream`. Para una playlist, `playlist.count` es el total público y
  `entries` una muestra acotada (máximo 200).
- `formats` llega deduplicado por altura: un solo candidato por altura, el mejor según muxed
  (vídeo+audio en un archivo), luego DASH video-only, luego el de mayor bitrate. `audio` va
  ordenado por bitrate descendente.
- `qualities` es la escalera agrupada que consume la extensión: como máximo `2160` (etiqueta
  `4K`), `1080`, `720`, `360` y una entrada `audio`, en ese orden y sólo cuando existen. Las
  demás alturas que reporta el backend (480, 240, …) se omiten en la escalera aunque sigan en
  `formats`. Cada entrada lleva `quality`, `label`, `height` (`null` para audio) y el
  `format_id` del candidato elegido para esa altura.
- `default_quality` vale `720` cuando está disponible; si no, la altura disponible más cercana
  (los empates resuelven hacia la mayor calidad); si no hay ninguna altura, `audio`.
- Los manifiestos HLS/DASH devuelven la misma escalera calculada sobre sus variantes o
  representaciones, sin entrada `audio`.
- El daemon valida esquema y host; sólo `https://`. Errores: `1004`, `1022`, `1023`, `1031`,
  `1040`-`1045`.

### 9.2. `media.add`

```json
{"method":"media.add","params":{"url":"https://www.youtube.com/watch?v=...",
 "mode":"video","height":720,"playlist_limit":100}}
```

- `mode` ∈ `video|audio|playlist`. `playlist_limit` se acota a 1–500 (default 100).
- En `mode: "video"` (y `"playlist"`) la calidad se elige **por `height`**: el daemon ignora
  `format_id` y compone el selector muxed/DASH por altura; `media_format` guarda `"720p"`.
  Sólo `mode: "audio"` usa `format_id`. Para manifiestos, `height` selecciona la variante o
  representación concreta en lugar de la mejor disponible.
- Devuelve `{"task_id":"...","status":"downloading","kind":"youtube_video"}`.
- `download_type` pasa a admitir `youtube_video`, `youtube_playlist`, `youtube_audio`, además de
  `direct`, `hls` y `dash`.
- Una playlist crea **una** tarea con `playlist_count`, `playlist_title` y `batch_id`; sus entradas
  se reportan en `playlist_progress {completed,total}`.

### 9.3. `media.status`

Acepta `{"task_id":"..."}` o `{"batch_id":"..."}` y devuelve estado, progreso, velocidad, ETA,
`title`, `final_path`, `error_code`, `error_message` y, en playlists, `playlist {completed,total}`.

### 9.4. Eventos

`download.event` de una tarea de medios agrega `media_kind` a `task_added` y
`playlist_progress {completed,total}` a `task_progress`. `seq_id` sigue siendo monótono por daemon.

### 9.5. Persistencia (esquema v2)

Migración idempotente a `PRAGMA user_version = 2` que agrega a `downloads`:
`media_kind TEXT`, `media_format TEXT`, `playlist_title TEXT`, `playlist_count INTEGER`,
`batch_id TEXT`, con `CREATE INDEX IF NOT EXISTS idx_downloads_batch ON downloads(batch_id)`.
Un archivo v1 migra en una transacción sin perder filas.

### 9.6. Seguridad

El host nativo sólo reenvía `media.*`, `system.*`, `download.*` y `settings.*`; cualquier otro método
responde `-32601` sin tocar el daemon. El daemon valida URL, host y límites antes de crear la tarea y
nunca registra query strings ni cabeceras sensibles.
