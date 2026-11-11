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
| `capture_extensions` | array[string] | lista de 04 | sin punto inicial |
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
1. **`task_added`**: Emitido cuando una nueva tarea ingresa a la cola.
2. **`state_changed`**: Emitido en cualquier transición de la máquina de estados.
3. **`segment_started`**: Un worker inicia la descarga de un bloque de bytes.
4. **`segment_split`**: Un segmento activo fue dividido en caliente (contiene `donor_id`, `new_segment_id`, `midpoint`).
5. **`segment_completed`**: Un worker finalizó exitosamente su rango de bytes.
6. **`task_progress`**: Emitido a una tasa máxima regulada de **4 eventos por segundo** por descarga activa para no saturar el bus de eventos ni el renderizado de la GUI. Contiene bytes acumulados, velocidad instantánea en bytes/s y ETA.
7. **`task_completed`**: Descarga terminada y validada en disco.
8. **`task_failed`**: Error terminal en la descarga (incluye código y mensaje legible).

### Payload mínimo por evento

| `event_type` | `data` obligatorio |
| --- | --- |
| `task_added` | `task_id`, `status`, `url`, `final_path` |
| `state_changed` | `old_state`, `new_state`, `error_code?` |
| `segment_started` | `segment_id`, `start_offset`, `end_offset` |
| `segment_split` | `donor_segment_id`, `new_segment_id`, `midpoint`, `original_end` |
| `segment_completed` | `segment_id`, `start_offset`, `end_offset` |
| `task_progress` | `downloaded_bytes`, `total_bytes?`, `speed_bps`, `eta_seconds?`, `segments[]` |
| `task_completed` | `final_path`, `total_bytes`, `checksum_ok` |
| `task_failed` | `error_code`, `message`, `retryable` |

`seq_id` es monótono por daemon, no por cliente. Un cliente que detecta un salto debe llamar a
`download.list` y descartar su caché; no se intenta reenviar eventos perdidos.
