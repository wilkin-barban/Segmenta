# OpenSpec 07: Contrato IPC, Protocolo JSON-RPC y Control de Ciclo de Vida

> **Especificación:** OPENSPEC-SEGMENTA-007  
> **Versión:** 1.1.0  
> **Área:** Comunicación Inter-Procesos (IPC), Seguridad de Sockets Unix y Catálogo de Eventos

> **Alcance de esta revisión.** El árbol de código está en **0.1.2** (`Cargo.toml`,
> `extension/*/manifest.json`). Todo lo que aquí aparece sin etiqueta **existe hoy** en
> `crates/` y `extension/`, y los nombres de método, parámetros, campos, valores y códigos
> son los del dispatcher real (`crates/segmenta-daemon/src/manager.rs`, `rpc.rs`, `ipc.rs`).
> Lo que pertenece a la release 0.2.0 se marca con **«(0.2, en preparación)»** y no debe
> leerse como comportamiento entregado; su contrato vive en el nuevo OpenSpec 12
> (`12-queue-scheduler-postprocess-auth.md`, todavía no presente en `docs/openspec/`).

---

## 1. Transporte, Autenticación y Ciclo de Vida del Socket

### 1.1. Ubicación y Permisos del Socket
* **Ruta del Socket Unix:** `$XDG_RUNTIME_DIR/segmenta/daemon.sock`
  (normalmente `/run/user/<UID>/segmenta/daemon.sock`). Si `$XDG_RUNTIME_DIR` no existe, el
  daemon y el host resuelven `/tmp/segmenta-<UID>/daemon.sock`; `SEGMENTA_SOCKET` y
  `segmenta-daemon --socket <ruta>` permiten sobrescribirla en desarrollo y en tests.
* **Permisos:** el directorio contenedor se crea con **`0700`** y el socket con **`0600`**
  (`IpcServer::bind`). En socket activation el inodo lo posee systemd
  (`SocketMode=0600`, `DirectoryMode=0700`) y el daemon no lo desvincula al salir.
* **Reset de socket obsoleto:** si la ruta ya existe antes del `bind`, el daemon elimina el
  archivo, para que un daemon caído no bloquee el arranque.

### 1.2. Autenticación de Pares (`SO_PEERCRED`)
1. Cada conexión aceptada consulta `stream.peer_cred()`.
2. **Validación:** `peer_uid == daemon_uid` (`ipc::peer_is_authorized`).
3. Si no coincide, el socket se cierra **sin leer ni escribir un solo byte** y se registra
   `warn` en el log. El código reservado para el cliente es `1010`
   (`UNAUTHORIZED_PEER`), pero la conexión se corta antes de poder responderle.

### 1.3. Unidades systemd y bloqueo de instancia única
`packaging/segmenta.socket` (instalado en `/usr/lib/systemd/user/`):

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

`packaging/segmenta.service` (instalado en `/usr/lib/systemd/user/`):

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

[Install]
WantedBy=default.target
```

* **Socket activation:** con `--systemd`, el daemon adopta el descriptor cuando
  `LISTEN_FDS == 1` y `LISTEN_PID` coincide con su PID (`ipc::systemd_listen_fd`); en
  cualquier otro caso crea el socket él mismo.
* **Instancia única:** independientemente de systemd, el daemon toma
  `flock(LOCK_EX | LOCK_NB)` sobre `$XDG_RUNTIME_DIR/segmenta/daemon.lock` y mantiene el
  descriptor abierto durante toda la vida del proceso. Si el bloqueo falla imprime
  `ERR_DAEMON_ALREADY_RUNNING` y termina con código de salida distinto de cero.
* El paquete **no** ejecuta `systemctl enable`: la unidad se habilita por el operador o por
  la GUI.

### 1.4. Versión del protocolo
* La versión es **1** (`ipc::PROTOCOL_VERSION`). `system.hello` responde
  `{"protocol_version": 1, "daemon_version": "<versión del binario>", "min_client_version": 1}`.
* **Lo que el daemon no hace hoy:** no inspecciona los parámetros de `system.hello`
  (`client_name`/`client_version` se aceptan y se ignoran), no rechaza clientes por versión
  (no existe `ERR_PROTOCOL_VERSION`) y no detecta `id` repetidos (no existe
  `ERR_DUPLICATE_REQUEST_ID`). La negociación es informativa: el cliente compara
  `protocol_version`/`min_client_version` y decide.
* Las notificaciones no llevan `id`; las respuestas conservan el `id` recibido. Un `id`
  ausente se responde como `null`.

## 2. Framing, Límites y Control de Flujo

1. **Framing:** JSON-RPC 2.0 delimitado por salto de línea (`\n`), NDJSON. Un `\r` final se
   tolera y se descarta. Una línea vacía no es un mensaje y se ignora.
2. **Tamaño máximo:** **1 MiB (1 048 576 bytes)** por mensaje (`MAX_MESSAGE_BYTES`). El frame
   se lee sin bufferizar más allá del límite; si lo excede, el daemon responde
   `1009 PAYLOAD_TOO_LARGE` y cierra la conexión.
3. **`jsonrpc` obligatorio:** si el campo no es exactamente `"2.0"` la respuesta es
   `-32600 INVALID_REQUEST` y la conexión sigue abierta.
4. **Método desconocido:** `-32601 METHOD_NOT_FOUND` con `data.code = "METHOD_NOT_FOUND"`.
5. **Desconexiones:** cerrar la GUI o la extensión **nunca** detiene las descargas activas;
   las tareas pertenecen al daemon.
6. **Buffer de eventos:** canal `broadcast` de **1024** eventos (`ipc::EVENT_BUFFER`). Un
   cliente que se atrasa más que el buffer recibe `system.resync_required` en lugar de
   eventos obsoletos y debe recargar con `download.list`.
7. **Forma del error:** todo error es `{"code": <número>, "message": "<texto>",
   "data": {"code": "ERR_*"}}`; `data` se omite sólo si el error no tiene nombre.

---

## 3. Catálogo Exhaustivo de Métodos JSON-RPC

Todos los mensajes declaran `"jsonrpc": "2.0"`. El dispatcher de
`crates/segmenta-daemon/src/manager.rs` (`impl RpcHandler for Manager`) acepta exactamente
los métodos de las secciones 3.1–3.11. Cualquier otro nombre termina en `-32601`: no hay
alias, ni métodos «en reserva», ni despacho por prefijo.

Lista cerrada de los **27** brazos del `match`, uno por uno:

```text
system.hello   system.status   system.shutdown   system.open
download.add   download.list   download.status   download.pause
download.resume   download.cancel   download.delete
media.probe   media.add   media.status
torrent.probe   torrent.add   torrent.status
extension.status   extension.prepare   extension.open   extension.install-native
settings.get   settings.update
queue.list   queue.create   queue.remove   queue.schedule
```

`site_login.*` (3.12) **no** está en ese `match`: hoy responde `-32601` y su contrato es de
0.2. `download.bin` **no** es un método (3.13).

### 3.1. `system.hello`, `system.status`, `system.shutdown`, `system.open`

* **`system.hello`** — parámetros ignorados. Respuesta:
  `{"protocol_version": 1, "daemon_version": "...", "min_client_version": 1}`.
* **`system.status`** — sin parámetros. Respuesta:
  ```json
  {
    "daemon_version": "0.1.2",
    "uptime_seconds": 3600,
    "memory_bytes": 18874368,
    "active_connections": 2,
    "event_subscribers": 1,
    "secret_backend": "keyring"
  }
  ```
  `memory_bytes` es el RSS leído de `/proc/self/statm` (mejor esfuerzo; `0` si no se puede
  leer). `active_connections` cuenta **tareas en ejecución**, no sockets conectados.
  `secret_backend` es `keyring` o `unavailable` (`secrets.rs`).
* **`system.shutdown`** — cancela todas las tareas en ejecución, espera a que terminen,
  envía la señal de apagado al bucle principal y responde `{"shutting_down": true}`. El
  proceso sale con código **0**.
* **`system.open`** — parámetros `{"task_id": "..."}`. Verifica que la tarea exista (`1001`)
  y lanza la GUI con argv fijo `segmenta-gui --show-task <task_id>`, sin shell. El binario
  se resuelve por `SEGMENTA_GUI` o como hermano del daemon. Respuesta:
  `{"opened": true, "task_id": "...", "gui_path": "/usr/bin/segmenta-gui"}`. Si no hay GUI
  ejecutable: `1053 ERR_GUI_NOT_FOUND`.

### 3.2. `download.add`
* **Parámetros:**
  * `url` (string, obligatorio): sólo `http`/`https`; otro esquema es `1004 ERR_INVALID_URL`.
  * `destination_dir` (string, opcional): ruta **absoluta**; si se omite se usa el
    `download_dir` vigente. No hay clasificación por categoría: `category_dirs` se persiste
    pero **no** se consulta aquí. Se crea si no existe; si no se puede crear o no es
    escribible: `1005 ERR_DESTINATION_UNWRITABLE`.
  * `filename` (string, opcional): se sanea (`sanitize_filename`). Si falta, se sugiere a
    partir de la URL; si tampoco hay nombre, se usa **`download.bin`**.
  * `headers` (object, opcional): mapa de cabeceras. `Authorization`, `Proxy-Authorization`
    y `Cookie` **no** van a SQLite: se guardan como secreto en el llavero (alias
    `download/<task_id>`). Si no hay backend de secretos, el alta falla con
    `1031 ERR_KEYRING_UNAVAILABLE` y **no** se crea la tarea.
  * `cookies` (array, opcional): `[{"name","value","domain","path"}]`. Se consolidan con
    las cabeceras sensibles en el mismo secreto.
  * `connections` (integer, opcional): se acota a **1–32**; por defecto
    `max_connections` de los ajustes.
  * `expected_checksum` (object, opcional): `{"algorithm":"sha256","hash":"..."}`. Otro
    algoritmo es `-32602`.
  * `start_immediately` (boolean, opcional, default `true`): decide si el daemon arranca la
    tarea en el acto.
  * `queue_name` (string, opcional, default `"main"`): la fila se inserta con
    `REFERENCES queues(name)`, así que un nombre inexistente falla por clave foránea y hoy
    se reporta como `-32603 INTERNAL_ERROR` (no hay código dedicado).
  * `priority` (integer, opcional, default `0`): se **persiste**; hoy ningún componente
    ordena por prioridad (el scheduler que lo consumirá es 0.2, ver 12).
* **Respuesta:**
  * `task_id` (string, UUIDv4).
  * `status` (string): estado persistido al responder; normalmente `"pending"`, porque el
    motor cambia de estado de forma asíncrona.
  * `filename` (string): nombre saneado.
  * `destination_dir` (string): **carpeta efectiva** (`destination_dir` del pedido o el
    `download_dir` vigente).
  * `final_path` (string): ruta absoluta **resuelta y desduplicada** que producirá la tarea
    (`manual (1).deb` cuando `manual.deb` ya existe).
* Efecto colateral: evento `task_added` con `task_id`, `status`, `url`, `destination_dir` y
  `final_path`. La ventana de confirmación de captura (§10) muestra exactamente ese par.

### 3.3. `download.list`
* **Parámetros:** `filter` (opcional: `all`, `active`, `paused`, `completed`, `failed`,
  `cancelled`, `pending`; otro valor es `-32602`), `limit` (default 100, acotado a 1–1000),
  `offset` (default 0).
* **Respuesta:** array de vistas sin segmentos (`segments: []`). Orden real:
  `created_at DESC, rowid DESC`; `limit`/`offset` se aplican después del filtro.
* Campos de cada vista: `task_id`, `url`, `final_path`, `filename`, `status`,
  `total_bytes`, `downloaded_bytes`, `speed_bps`, `eta_seconds`, `retry_count`,
  `supports_ranges`, `queue_name`, `created_at`, `completed_at`, `error_code`,
  `error_message`, `media_kind`, `playlist_title`.

### 3.4. `download.status`
* **Parámetros:** `{"task_id": "..."}`; inexistente es `1001`.
* **Respuesta:** la misma vista de 3.3 **con** `segments[]` (`id`, `start_offset`,
  `end_offset`, `next_offset`, `status` ∈ `idle`/`active`/`completed`/`failed`,
  `speed_bps`). Si hay snapshot de progreso en memoria se usa ése; si no, se leen los
  segmentos persistidos. **El `speed_bps` de cada segmento vale siempre `0` hoy**, tanto en
  la rama del snapshot como en la leída de SQLite; el `speed_bps` de la tarea (nivel
  superior) sí viene del snapshot. El estado de cada segmento se deriva: `completed` si
  `next_offset > end_offset`, `active` sólo si la tarea está `downloading`, `idle` en el
  resto de los casos.

### 3.5. `download.pause` y `download.resume`
* **`download.pause`** — `{"task_id": "..."}`. Sólo se puede pausar una tarea en
  `downloading`, `pending`, `probing`, `retrying`, `verifying` o `finalizing`; en cualquier
  otro estado responde `1011 TASK_NOT_PAUSABLE` sin tocar la fila. Cancela el worker,
  espera su checkpoint y responde `{"task_id": "...", "status": "<estado real>"}` (el
  motor deja `paused`).
* **`download.resume`** — `{"task_id": "..."}`. Sólo acepta `paused` y `retrying`: en
  `downloading` es `1002 TASK_ALREADY_ACTIVE`, en cualquier otro estado
  `1003 TASK_NOT_RESUMABLE`. Relanza el motor con `max_connections` de los ajustes y
  responde `{"task_id": "...", "status": "downloading"}`.

### 3.6. `download.cancel` y `download.delete`
* **`download.cancel`** — `{"task_id": "...", "delete_file": false}`. Cancela el worker si
  corre, marca la tarea `cancelled`, borra el secreto asociado del llavero y, con
  `delete_file: true`, elimina el archivo parcial. Respuesta:
  `{"task_id": "...", "status": "cancelled", "file_deleted": <bool>}`; `file_deleted` es
  `true` sólo si el archivo existía y se borró.
* **`download.delete`** — `{"task_id": "...", "delete_file": <bool obligatorio>}`; omitirlo
  es `-32602`. Borra la fila (las tablas hijas caen por `ON DELETE CASCADE`), el secreto y
  el snapshot de progreso, y opcionalmente el archivo. Respuesta:
  `{"task_id": "...", "deleted": <bool>}` (si la fila existía).

### 3.7. `media.probe`, `media.add` y `media.status`
Contrato detallado en §9. Resumen de parámetros:
* `media.probe`: `{"url", "kind"?: "video"|"playlist"|"stream", "limit"?}`.
* `media.add`: `{"url", "mode": "video"|"audio"|"playlist", "format_id"?, "height"?,
  "playlist_limit"?}`. Responde `{"task_id", "status": "downloading", "kind",
  "batch_id"}` en la vía YouTube (`batch_id` es `null` salvo playlist) y
  `{"task_id", "status": "downloading", "kind"}` en la vía de manifiestos HLS/DASH, donde
  `kind` es el del manifiesto (`hls`/`dash`).
* `media.status`: `{"task_id"}` **o** `{"batch_id"}`; con ambos gana `task_id`; sin ninguno
  es `-32602`. Devuelve la vista de 3.4 más `media_kind`, `media_format`, `playlist_title`,
  `playlist_count` y `batch_id`.

### 3.8. `torrent.probe`, `torrent.add` y `torrent.status`
Los tres reciben el objeto JSON crudo y validan campo por campo (no usan struct tipado).

* **`torrent.probe`** — `{"url": "magnet:..."}` o URL `http(s)` de `.torrent`. La respuesta
  la construye `crate::torrent::probe_torrent` (fuera del alcance verificado de esta
  revisión); el contrato que sí consume el daemon es el del párrafo siguiente.
  * Magnet: `{"name": "<nombre o magnet_download>", "info_hash": "", "total_bytes": null,
    "files": [], "source_type": "magnet"}`. No se resuelven metadatos: hasta que la sesión
    los tenga, no hay tamaño ni archivos. Magnet inválido: `-32602`.
  * `.torrent`: se descarga con el cliente HTTP del daemon y se decodifica con
    `librqbit_bencode`; el probe multiarchivo (R2, ya en el árbol) arma `files[]` y el
    `total_bytes`. De esa respuesta, `torrent.add` sólo consume `name`, `info_hash`,
    `total_bytes` y `source_type`. Error de red: `1004` con nombre `ERR_NETWORK`; bencode
    inválido: `-32602`.
  * **Límite implementado en `torrent.add`, no en el probe:** el probe multiarchivo (R2) ya
    está en el árbol, pero `torrent.add` sólo lee `name`, `info_hash`, `total_bytes` y
    `source_type` de la respuesta e ignora `files[]`: escribe `files_count = 1` y **una**
    fila de `torrent_files` con índice `0` y el tamaño total. El detalle por archivo en la
    tarea sigue pendiente **(0.2, en preparación)**.
* **`torrent.add`** — `{"url" (obligatorio), "destination_dir"? (absoluta, default
  `download_dir`), "seed_after_complete"? (default `false`)}`. Persiste la tarea con
  `download_type` `torrent` o `magnet`, inserta la fila de `torrents` y **una** fila de
  `torrent_files` (índice `0`), y lanza el trabajo en la sesión `librqbit` compartida.
  Respuesta: `{"task_id", "status": "downloading", "filename", "info_hash",
  "destination_dir", "final_path"}` (mismo par efectivo que `download.add`).
  * El nombre y el tamaño provienen del probe; `info_hash` cae a
    `0000000000000000000000000000000000000000` cuando el probe devuelve cadena vacía.
  * `files_count` de `torrents` se escribe como `1`, coherente con el límite anterior.
  * Rutas internas: `torrent::sanitize_torrent_path` rechaza rutas absolutas, `..`, `.` y
    componentes inválidos contra `destination_dir`; el fallo es `-32602`.
  * **`seed_after_complete` se persiste pero todavía no se honra:** `run_torrent` termina en
    cuanto el payload está completo y cancelar borra el torrent de la sesión sin tocar los
    datos.
* **`torrent.status`** — `{"task_id"}`. Devuelve la vista de 3.4 más `info_hash`,
  `seed_after_complete`, `uploaded_bytes`, `peers`, `files_count` (si hay fila en
  `torrents`), `files[]` (de `torrent_files`) y `destination_dir` (derivado del
  `final_path`).

### 3.9. `extension.status`, `extension.prepare`, `extension.open`, `extension.install-native`
* **`extension.status`** — sin parámetros. Respuesta:
  ```json
  {
    "extension_id": "kdeldnifdbpemnhfpmhonlnibbhnoaan",
    "firefox_extension_id": "segmenta@wilkin.github.io",
    "host_path": "/usr/bin/segmenta-host",
    "extension_version": "0.1.2",
    "shipped_archives": {"chromium": "/usr/share/segmenta/extensions/segmenta-chromium.zip",
                         "firefox": "/usr/share/segmenta/extensions/segmenta-firefox.zip"},
    "browsers": [
      {"id": "chrome", "name": "Google Chrome", "family": "chromium",
       "command": "google-chrome", "version": "…", "unsupported": null,
       "manifest_path": "/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json",
       "manifest_present": true, "user_manifest_present": false,
       "extension_installed": false, "prepared_path": null, "archive_path": "…",
       "state": "native-ready"}
    ]
  }
  ```
  `shipped_archives.*` y `prepared_path`/`archive_path` pueden ser `null`. `command` es
  `null` si el ejecutable no está en `PATH`. `unsupported` vale `"snap"` cuando el
  ejecutable vive bajo `/snap`. `state` es uno de `unsupported`, `not-installed`,
  `extension-installed`, `native-ready`, `native-missing`, evaluados en ese orden.
  `extension_installed` es una lectura del perfil del navegador
  (`<perfil>/Extensions/<id>` en Chromium, `<perfil>/extensions/<gecko-id>*` en Firefox);
  el daemon nunca escribe en un perfil.
* **`extension.prepare`** — `{"family"?: "chromium"|"firefox"|"all"}` (default `all`).
  Descomprime el ZIP instalado en
  `$XDG_DATA_HOME/segmenta/extensions/<familia>-<versión>` y responde
  `{"prepared": [{"family": "chromium", "path": "..."}]}`. Familia desconocida o archivo
  ausente/corrupto: `1051 ERR_EXTENSION_PREPARE`.
* **`extension.open`** — `{"browser_id": "chrome"}` (se acepta también la clave `browser`).
  Sin `browser_id` es `-32602`; si el navegador no se puede lanzar,
  `1052 ERR_EXTENSION_OPEN`. Respuesta: `{"opened": true, "browser_id": "chrome"}`.
* **`extension.install-native`** — sin parámetros. Recorre los navegadores detectados con
  ejecutable y escribe el manifiesto Native Messaging **de usuario** (nunca sobrescribe uno
  existente). Respuesta:
  `{"installed": [{"id": "chrome", "path": "..."}], "errors": [{"id": "chromium",
  "message": "..."}]}`. Sin `HOME`: `1050 ERR_NO_HOME`. Un paquete snap aparece en `errors`
  con el motivo, no en `installed`.

### 3.10. `settings.get` y `settings.update`
* **`settings.get`** — sin parámetros. Devuelve el diccionario completo con los valores
  efectivos (tabla de 3.10.1). Los secretos (proxy con contraseña, `Authorization`,
  cookies, contraseñas de sitios) **no** aparecen: viven en el llavero (08).
* **`settings.update`** — un objeto parcial. Una clave desconocida es `-32602` con
  `unknown setting "<clave>"`; un tipo o rango inválido también es `-32602`. El objeto se
  valida entero antes de persistir: si falla, no se guarda nada. Devuelve el diccionario
  completo resultante.
* Los ajustes se guardan como filas `settings(key, value_json)`. En la misma tabla vive
  `queue_schedule:<cola>`, que **no** forma parte de `settings.get`.

#### 3.10.1. Catálogo real de settings

| Clave | Tipo | Default | Validación / nota |
| --- | --- | --- | --- |
| `max_connections` | integer | 16 | 1–32 |
| `global_speed_limit_bps` | integer/null | null | máximo 1 GiB/s; `null` = ilimitado |
| `clip_monitor_enabled` | boolean | true | se persiste; la GUI todavía no lo respeta al prellenar (U1, 0.2) |
| `autostart` | boolean | false | systemd --user |
| `download_dir` | string | `$XDG_DOWNLOAD_DIR`, `xdg-user-dir DOWNLOAD`, `~/.config/user-dirs.dirs`, `$HOME/Downloads` | ruta absoluta |
| `category_dirs` | object | `{}` | categoría → ruta absoluta. Se persiste y se valida, pero **nada lo consume**; sin categorías automáticas (0.2 retira la UI, conserva la lectura legacy) |
| `capture_enabled` | boolean | true | interruptor maestro de la captura del navegador |
| `capture_extensions` | array[string] | 64 tokens (ver nota) | máximo 200 entradas; cada token pasa `normalize_extension` (sin punto inicial) |
| `capture_torrents` | boolean | true | captura `.torrent` |
| `capture_magnet` | boolean | true | captura enlaces `magnet:` |
| `capture_confirm` | boolean | true | pausa la descarga del navegador y abre la ventana de confirmación con nombre, host y carpeta efectiva; `false` captura sin preguntar |
| `domain_denylist` | array[string] | `[]` | máximo 500; host exacto o sufijo explícito, sólo `[A-Za-z0-9.:-]` |
| `schedule_start` | string/null | null | `HH:MM`; **no** lleva zona horaria ni se aplica hoy; scheduler **(0.2, en preparación)** (§11) |
| `schedule_stop` | string/null | null | `HH:MM`; ídem |
| `action_on_complete` | string | `none` | `none`/`suspend`/`power_off`/`exit`; se persiste y hoy no se ejecuta; ejecución **(0.2, en preparación)** (§11) |
| `allow_local_downloads` | boolean | false | habilita explícitamente destinos loopback/LAN (SSRF) |
| `max_retries` | integer | 5 | 0–20 |
| `proxy_mode` | string | `system` | `system`/`none`/`manual` |
| `proxy_url` | string/null | null | obligatoria y `http(s)://`/`socks5://` cuando `proxy_mode = manual` |
| `postprocess_command` | array[string] | `[]` | argv sin shell; **se persiste y hoy no se ejecuta** (§11) |
| `appearance` | string | `system` | `system`/`light`/`dark` |

**Nota sobre `capture_extensions`:** el default de 0.1.2 tiene 64 tokens (incluye `deb`,
`rpm`, `appimage`, `whl`, `jar`, `epub`, `vsix`, `xpi`, `msix`, `xz`, patrones `r0*`/`r1*`
y formatos de música/video). Una fila escrita por 0.1.0 o 0.1.1 cuya lista sea byte a byte
su default histórico se actualiza **por unión** (primero los defaults de 0.1.2, luego los
tokens que sólo tenía esa lista: `dmg`, `flac`, `tgz`, `webm` en 0.1.0), de forma
idempotente; una lista personalizada no se toca.

**(0.2, en preparación) — ajustes planificados, no aceptados hoy:**

* `antivirus_enabled` (default `false`), `antivirus_timeout_secs` (default `120`) y
  `antivirus_quarantine_dir` (default `$XDG_DATA_HOME/segmenta/quarantine`). **No existen
  todavía**: hoy `settings.update` los rechaza con `-32602 unknown setting`. Su contrato
  está en el nuevo OpenSpec 12; no se documentan aquí como disponibles.
* `site_login.*` tampoco son claves de `settings`: son métodos RPC (3.12) y credenciales en
  el llavero (08 §5), ambos **(0.2, en preparación)**.
* Las claves de planificación sí existen en `SETTING_KEYS` (`schedule_start`,
  `schedule_stop`, `action_on_complete`) y la ventana por cola se guarda en
  `queue_schedule:<cola>` (3.11), pero **nada las aplica**: el scheduler nativo que las
  consume es **(0.2, en preparación)** (§11).

### 3.11. `queue.list`, `queue.create`, `queue.remove`, `queue.schedule`
* **`queue.list`** — sin parámetros. Respuesta:
  `[{"name": "main", "max_active": 3, "active": 2, "pending": 5, "paused": 1}]`, ordenada
  por nombre. `active` cuenta estado `downloading`, `pending` cuenta `pending`, `paused`
  cuenta `paused`; los contadores se calculan recorriendo hasta 1000 tareas, no con
  agregados SQL.
* **`queue.create`** — `{"name": "noche", "max_active": 1}`. Nombre único de 1–64
  caracteres `[a-z0-9_-]`; `max_active` **1–8**. Duplicado o inválido: `-32602`. Respuesta:
  `{"name": "...", "max_active": <n>}`.
* **`queue.remove`** — `{"name": "noche"}`. `main` no se puede eliminar. Si la cola tiene
  tareas en `downloading`/`probing`/`retrying`/`verifying`/`finalizing` responde
  `1001 QUEUE_IN_USE` (mismo número que `TASK_NOT_FOUND`, nombre distinto en `data.code`).
  Todo `StoreError::Queue` se traduce a ese mismo `1001 QUEUE_IN_USE`, así que borrar
  `main` o una cola inexistente responde igual. Las tareas restantes se mueven a `main`.
  Respuesta: `{"name": "...", "removed": true}`.
* **`queue.schedule`** — `{"queue_name", "start_time"?: "HH:MM"|null, "stop_time"?:
  "HH:MM"|null, "action_on_complete"?: "none"}`. La cola debe existir. Persiste
  `queue_schedule:<cola>` con `{"start_time","stop_time","action_on_complete"}` en la tabla
  `settings` y responde `{"queue_name": "...", "schedule": {...}}`.
  **Límite implementado:** el daemon **guarda** la ventana pero **no** la aplica; no hay
  loop que abra/cierre ventanas ni que honre `action_on_complete`, y `max_active` no limita
  el arranque de pendientes. Todo eso es el scheduler de 0.2 (§11 y el nuevo OpenSpec 12).
  `action_on_complete` no se valida en esta llamada: se acepta cualquier cadena y sólo
  `settings.update` valida el enum.

### 3.12. `site_login.list`, `site_login.set`, `site_login.remove` — (0.2, en preparación)
**No existen todavía:** no son brazos de `Manager::call` y hoy responden `-32601
METHOD_NOT_FOUND`. La tabla `site_logins` ya está en el esquema (§08 §4.2), pero ningún
código la lee ni la escribe. Contrato planificado en el nuevo OpenSpec 12
(`12-queue-scheduler-postprocess-auth.md`, aún no presente) §5.

### 3.13. Nombres que no son métodos
* **`download.bin` no es un método RPC.** Es únicamente el nombre de archivo de reserva
  cuando ni el pedido ni la URL aportan uno (`manager.rs`, `add_download`; `torrent.add` usa
  el mismo literal para el nombre del payload). El cliente de demostración de la GUI repite
  ese literal en su respuesta simulada. No existe `download.bin`, `download.*` genérico ni
  despacho por prefijo.
* **No hay métodos retirados:** los 27 nombres de 3.1–3.11 son el catálogo completo. Lo
  único ausente es `site_login.*`, que nunca existió (planificado, 3.12).

---

## 4. Tabla de Códigos de Error

Los códigos vienen del dispatcher (`rpc.rs`), del motor (`segmenta_core::CoreError`) y de
las capas de medios/extensión. `data.code` lleva siempre el nombre.

| Código | Identificador | Causa y acción |
| :--- | :--- | :--- |
| **`-32700`** | `PARSE_ERROR` | Frame que no es JSON válido o no es UTF-8. |
| **`-32600`** | `INVALID_REQUEST` | `jsonrpc` distinto de `"2.0"`. |
| **`-32601`** | `METHOD_NOT_FOUND` | Método no registrado. También es la respuesta que el host nativo da a un prefijo no autorizado (§7). |
| **`-32602`** | `INVALID_PARAMS` | Parámetros faltantes, tipos incorrectos, valores fuera de rango, `settings` desconocido, magnet/bencode/manifiesto inválido. |
| **`-32603`** | `INTERNAL_ERROR` | Fallo no controlado (SQLite, E/S, serialización). |
| **`1001`** | `TASK_NOT_FOUND` / `QUEUE_IN_USE` | El `task_id` no existe; o la cola a borrar todavía tiene tareas activas. El nombre distingue el caso. |
| **`1002`** | `TASK_ALREADY_ACTIVE` | `download.resume` sobre una tarea ya `downloading`. |
| **`1003`** | `TASK_NOT_RESUMABLE` | La tarea no está en `paused`/`retrying`. |
| **`1004`** | `INVALID_URL` / `ERR_NETWORK` | Esquema distinto de `http(s)`; o fallo de red al descargar un `.torrent`. |
| **`1005`** | `DESTINATION_UNWRITABLE` | La carpeta destino no se puede crear ni usar. |
| **`1006`** | `DISK_FULL` | `ENOSPC` al preasignar o escribir. |
| **`1007`** | `ERR_BLOCKED_PRIVATE_IP` | La URL resuelve a una IP privada/loopback y `allow_local_downloads` está en `false`. |
| **`1008`** | `CHECKSUM_MISMATCH` | El digest final no coincide con `expected_checksum`. |
| **`1009`** | `PAYLOAD_TOO_LARGE` | Frame de más de 1 MiB. |
| **`1010`** | `UNAUTHORIZED_PEER` | Reservado; el socket se corta antes de responder. |
| **`1011`** | `TASK_NOT_PAUSABLE` | `download.pause` sobre una tarea en estado terminal. |
| **`1021`** | `ERR_TOO_MANY_REDIRECTS` | Se agotó el límite de redirecciones. |
| **`1022`** | `ERR_RESOURCE_GONE` | El servidor responde 410. |
| **`1023`** | `ERR_AUTH_REQUIRED` | El servidor pide autenticación y no hay credenciales utilizables. |
| **`1024`** | `ERR_PROXY_AUTH` | El proxy exige autenticación. |
| **`1025`** | `ERR_RANGE_OUT_OF_BOUNDS` | El rango pedido cae fuera del recurso. |
| **`1026`** | `ERR_SIZE_MISMATCH` | El tamaño final no coincide con el declarado. |
| **`1027`** | `ERR_TRUNCATED_RESPONSE` | El cuerpo terminó antes de lo anunciado. |
| **`1028`** | `ERR_UNSUPPORTED` | Función o servidor no soportado. |
| **`1029`** | `ERR_NETWORK` / `ERR_DAEMON_UNAVAILABLE` | Fallo de red del motor; el host nativo usa el mismo número cuando no alcanza el daemon. |
| **`1030`** | `ERR_CANCELLED` | El worker se canceló. |
| **`1031`** | `ERR_KEYRING_UNAVAILABLE` | No hay backend de secretos y la operación necesita uno. |
| **`1032`** | `ERR_MALFORMED_HEADER` | Cabecera HTTP inválida. |
| **`1040`** | `ERR_DRM_PROTECTED` | El manifiesto HLS trae `EXT-X-KEY` distinto de `NONE` o el MPD trae `ContentProtection`. |
| **`1041`** | `ERR_LIVE_NOT_SUPPORTED` | Playlist o MPD en vivo. |
| **`1042`** | `ERR_MANIFEST_MALFORMED` | Manifiesto HLS/DASH ilegible o inválido. |
| **`1043`** | `ERR_STREAM_UNSUPPORTED` | Variante o representación fuera de alcance. |
| **`1044`** | `ERR_MEDIA_BACKEND` / `ERR_TORRENT_SESSION` | Fallo del backend de medios (`yt-dlp`); el motor torrent reutiliza el número para el fallo de sesión. |
| **`1045`** | `ERR_MEDIA_NOT_PUBLIC` | Contenido no público (privado, con edad o no accesible sin sesión). |
| **`1050`** | `ERR_NO_HOME` | `extension.install-native` sin `HOME`. |
| **`1051`** | `ERR_EXTENSION_PREPARE` | No se pudo preparar el ZIP de la extensión. |
| **`1052`** | `ERR_EXTENSION_OPEN` | No se pudo abrir el navegador. |
| **`1053`** | `ERR_GUI_NOT_FOUND` | `system.open` sin GUI ejecutable. |

**(implementado en `main`, entra en 0.2.0)** `1046 ERR_ANTIVIRUS_FOUND` y
`1047 ERR_ANTIVIRUS_UNAVAILABLE`, documentados en 12. El escaneo, la cuarentena y la migración
v4 ya existen en el árbol; la release sigue sin publicarse.

---

## 5. Catálogo de Notificaciones Push y Sincronización de Eventos

Todos los eventos viajan como notificación `download.event` con un `seq_id` monótono por
daemon:

```json
{
  "jsonrpc": "2.0",
  "method": "download.event",
  "params": {
    "seq_id": 4821,
    "timestamp_ms": 1794356400000,
    "event_type": "state_changed",
    "task_id": "c1f7a28e-...",
    "data": { "old_state": "probing", "new_state": "downloading" }
  }
}
```

### 5.1. Eventos emitidos hoy

| `event_type` | Cuándo | `data` mínimo |
| --- | --- | --- |
| `task_added` | alta por `download.add`, `media.add` o `torrent.add` | `task_id`, `status`, `url`, `destination_dir`, `final_path` |
| `task_started` | arranque del trabajo torrent | `status` |
| `state_changed` | toda transición de estado | `old_state`, `new_state`, y según el caso `total_bytes`, `supports_ranges`, `etag`, `last_modified`, `reason` |
| `segment_started` | un worker empieza un rango | `segment_id`, `start_offset`, `end_offset` |
| `segment_split` | hot-split aceptado | `donor_segment_id`, `new_segment_id`, `midpoint`, `original_end` |
| `segment_completed` | un worker termina su rango | `segment_id`, `start_offset`, `end_offset` |
| `task_progress` | como máximo **4 veces por segundo** y por descarga (`PROGRESS_INTERVAL = 250 ms`) | `downloaded_bytes`, `total_bytes`, `speed_bps`, `eta_seconds`, `segments[]` |
| `task_completed` | descarga finalizada | `final_path`, `total_bytes`, `checksum_ok`; los trabajos de medios y torrent agregan `media` |
| `task_cancelled` | cancelación reportada por el motor | `error_code`, `message`, `retryable` |
| `task_failed` | error terminal | `error_code`, `message`, `retryable` |
| `download_restarted_due_to_stale_cache` | el servidor devolvió `200` a una reanudación con `If-Range` o cambió el total: el motor truncó y reinició desde cero (`06` §2.2) | `reason` |

`seq_id` es monótono por daemon, no por cliente. Un cliente que detecta un salto debe
llamar a `download.list` y descartar su caché: no se reenvían eventos perdidos.

`system.resync_required` no es un evento de tarea: es la notificación que el servidor envía
cuando el `broadcast` descartó eventos para ese cliente
(`{"reason": "event buffer overflow; reload with download.list"}`).

**(0.2, en preparación)** los estados `verifying` de post-proceso y las transiciones a
cuarentena no existen todavía; su contrato está en 12.

---

## 6. Relación entre el daemon y la GUI/CLI

* La GUI y la CLI son clientes JSON-RPC puros: no abren SQLite ni hacen HTTP por su cuenta.
* La GUI se suscribe a `download.event` y reconcilia con `download.list`/`download.status`
  cuando recibe `system.resync_required`.
* `system.open` es el único punto donde el daemon lanza otro proceso, y lo hace con argv
  fijo (`--show-task <id>`), sin shell.

---

## 7. Allowlist del host nativo (`segmenta-host`)

El host de Native Messaging reenvía al daemon **sólo** los métodos cuyo prefijo pertenece a
esta lista (`crates/segmenta-host/src/proxy.rs`, `ALLOWED_METHOD_PREFIXES`, cinco entradas en
este orden exacto):

```rust
pub const ALLOWED_METHOD_PREFIXES: [&str; 5] =
    ["media.", "system.", "download.", "settings.", "torrent."];
```

* No hay un sexto prefijo: `extension.` y `queue.` **no** están en la allowlist (el
  comentario de cabecera de `proxy.rs` todavía nombra sólo cuatro; manda el array).
* Un método fuera de esos prefijos se responde **localmente** con `-32601` y no sale del
  proceso.
* El nombre posterior al prefijo debe ser un identificador con puntos: sólo caracteres
  ASCII alfanuméricos, `_` y `.`. Un prefijo seguido de espacio, `/` o byte de control se
  rechaza igual.
* `extension.*`, `queue.*` y `site_login.*` **no** están en la allowlist: el navegador no
  puede consultar ni cambiar colas, extensión ni credenciales.
* El host no acepta comandos, rutas ni argv del navegador; el único proceso externo que
  puede lanzar es `systemctl --user start segmenta.socket` con argv fijo.
* Framing: 4 bytes little-endian de longitud + UTF-8 JSON, máximo 1 MiB, respuesta en el
  mismo framing. Errores propios del framing: `ERR_FRAME_INVALID_JSON`,
  `ERR_FRAME_TOO_LARGE`.

---

## 8. Coherencia de estados

Los estados persistidos son exactamente los del `CHECK` de `downloads`:
`pending`, `probing`, `downloading`, `paused`, `retrying`, `verifying`, `finalizing`,
`completed`, `failed`, `cancelled`. Ningún método inventa un estado fuera de esa lista; el
valor que un cliente debe esperar en `status` es siempre uno de esos diez.

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
- El daemon valida esquema y host; sólo `https://` y sin credenciales embebidas. Un host
  YouTube se resuelve con `yt-dlp` (`media.locate_tool` + `check_version`); el resto se parsea
  como manifiesto HLS/DASH. Errores: `-32602` (URL/límite inválido), `1040`
  `ERR_DRM_PROTECTED`, `1041` `ERR_LIVE_NOT_SUPPORTED`, `1042` `ERR_MANIFEST_MALFORMED`,
  `1043` `ERR_STREAM_UNSUPPORTED`, `1044` `ERR_MEDIA_BACKEND`, `1045` `ERR_MEDIA_NOT_PUBLIC`.

### 9.2. `media.add`

```json
{"method":"media.add","params":{"url":"https://www.youtube.com/watch?v=...",
 "mode":"video","height":720,"playlist_limit":100}}
```

- `mode` ∈ `video|audio|playlist`; otro valor es `-32602`. `playlist_limit` se acota a 1–500
  (default 100); `0` es `-32602`.
- En `mode: "video"` (y `"playlist"`) la calidad se elige **por `height`**: el daemon ignora
  `format_id` y compone el selector muxed/DASH por altura; `media_format` guarda `"720p"`.
  Sólo `mode: "audio"` usa `format_id`. Para manifiestos, `height` selecciona la variante o
  representación concreta en lugar de la mejor disponible.
- Devuelve `{"task_id":"...","status":"downloading","kind":"youtube_video","batch_id":null}`
  (`batch_id` sólo lleva valor en `mode: "playlist"`). La vía de manifiestos responde sin
  `batch_id` y con el `kind` del manifiesto.
- `download_type` admite `youtube_video`, `youtube_playlist`, `youtube_audio`, además de
  `direct`, `hls`, `dash`, `torrent` y `magnet`.
- Una playlist crea **una** tarea con `playlist_count`, `playlist_title` y `batch_id`; sus
  entradas se reportan en `playlist_progress {completed,total}`.

### 9.3. `media.status`

Acepta `{"task_id":"..."}` o `{"batch_id":"..."}` y devuelve estado, progreso, velocidad, ETA,
`title`, `final_path`, `error_code`, `error_message`, `media_kind`, `media_format`,
`playlist_title`, `playlist_count`, `batch_id` y, en playlists, `playlist {completed,total}`.

### 9.4. Eventos

`download.event` de una tarea de medios agrega `media_kind` a `task_added` y
`playlist_progress {completed,total}` a `task_progress`. `seq_id` sigue siendo monótono por daemon.

### 9.5. Persistencia (esquema v2, hoy v3)

Las columnas de medios (`media_kind`, `media_format`, `playlist_title`, `playlist_count`,
`batch_id`) e `idx_downloads_batch` se agregaron en la migración v2; el DDL vigente y la
cadena de migraciones están en `08-persistence-recovery.md` §4.

### 9.6. Seguridad

El host nativo reenvía `media.*`, `system.*`, `download.*`, `settings.*` y `torrent.*`
(§7); cualquier otro método responde `-32601` sin tocar el daemon. El daemon valida URL,
host y límites antes de crear la tarea y nunca registra query strings ni cabeceras sensibles.

---

## 10. Captura de archivos: perro guardián de la pausa (0.1.2)

La captura de archivos (`S10`) nunca cancela la descarga antes de confirmar y nunca deja una
descarga del navegador pausada sin un vigilante que sobreviva a la suspensión del service
worker de MV3.

### 10.1. Orden obligatorio

1. `downloads.onCreated` valida extensión, host y settings (`capture_enabled`,
   `capture_torrents`, `capture_magnet`, `capture_extensions`, `domain_denylist`).
2. **Antes de pausar**, la extensión escribe el registro pendiente en `storage.local` y crea la
   alarma periódica. Si cualquiera de las dos cosas falla, **no se pausa**: la descarga del
   navegador queda intacta y la captura se omite (regla a prueba de fallos). Nunca se pausa sin
   vigilante vivo.
3. `downloads.pause` y, con `capture_confirm: true`, `windows.create` sobre `confirm.html` con
   nombre, host y carpeta efectiva.
4. Aceptar: `download.add`/`torrent.add`, espera acotada a que la tarea demuestre bytes y sólo
   entonces `downloads.cancel` + `downloads.erase`; después se enfoca la tarea. Rechazar,
   timeout, cierre de la ventana o fallo del host: `downloads.resume` y la copia del navegador
   se conserva.
5. Toda salida (aceptar, rechazar, cierre, timeout, fallo del host) borra el registro pendiente
   y, si no queda ninguno, borra la alarma: no hay registros ni alarmas huérfanas.

### 10.2. Registro pendiente

Clave versionada `segmenta_pending_captures_v1` con un array de registros; cada registro lleva
su propio esquema (`v: 1`) además de `downloadId`, `url`, `filename`, `referrer`, `windowId` y
`deadline` (milisegundos epoch). La versión del esquema se declara en una constante explícita
del `background`; un registro con otra versión, sin `deadline` válido o con un `downloadId` que
no es entero se considera ilegible y se repara igual que uno expirado.

### 10.3. Vigilante (`alarms`)

* Permiso declarado en ambos manifiestos: **`alarms`** (Chromium y Firefox). Es el único
  temporizador que el navegador conserva cuando suspende el worker.
* Alarma única `segmenta-capture-watchdog` con `delayInMinutes: 1` y `periodInMinutes: 1`
  (mínimo de producción de Chrome). El timeout de decisión de ~120 s sigue siendo el
  `deadline` que viaja en el registro, no el único temporizador.
* `alarms.onAlarm` rehidrata los registros y cierra el trabajo de cada uno: si la ventana sigue
  existiendo y el `deadline` no venció, no toca nada; si el `deadline` venció, la ventana ya no
  existe o el registro es ilegible, reanuda la descarga y limpia. Si quedan registros, vuelve a
  crear la alarma.
* `runtime.onStartup` ejecuta el mismo barrido, para que un arranque nuevo del navegador cure
  cualquier registro que haya quedado de la sesión anterior.
* Una versión nueva de la caché de settings (`segmenta_settings_version`) obliga a releer
  `settings.get` en vez de responder con los valores de la versión anterior.

**Límite implementado (R1, 0.2 en preparación):** hoy no hay reconciliación al arrancar el
worker con `downloads.search`; una descarga que empieza y termina mientras el worker duerme
puede no capturarse. La reconciliación de cold-start es la tarea R1.

Consecuencia aceptada: entre el vencimiento del `deadline` y el siguiente tick de la alarma
(hasta un minuto), la descarga puede seguir pausada. Nunca se pierde el archivo: la copia del
navegador se reanuda en el primer tick que la alarma tenga ocasión de ejecutar.

---

## 11. Lo que el catálogo todavía no hace (0.2, en preparación)

Ninguno de estos puntos está implementado; el contrato completo está en el nuevo OpenSpec
12 (`12-queue-scheduler-postprocess-auth.md`, aún no presente en `docs/openspec/`).

* `queue.schedule` se persiste pero no se aplica; no hay scheduler nativo ni ventanas
  horarias efectivas.
* `max_active` se valida y se guarda, pero no limita cuántas tareas arranca el daemon: hoy el
  único límite real es el arranque explícito por `download.add`/`download.resume`/
  `media.add`/`torrent.add`.
* `action_on_complete` se persiste y no se ejecuta.
* `postprocess_command` se persiste y no se lanza; no hay hook argv, ni timeout, ni
  `postprocess_exit`/`postprocess_message`.
* Ya hay antivirus, cuarentena y `ERR_ANTIVIRUS_FOUND` (1046) más `ERR_ANTIVIRUS_UNAVAILABLE` (1047) en `main`, sin publicar.
* No hay `site_login.*`, ni inyección de `Authorization: Basic`, ni autenticación Digest (fuera
  de alcance por decisión).
* `priority` se persiste y ningún componente lo consume.
