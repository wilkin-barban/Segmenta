# OpenSpec 06: Contrato HTTP, Resiliencia y Reanudación

> **Especificación:** OPENSPEC-SEGMENTA-006  
> **Versión:** 1.0.0  
> **Área:** Protocolo HTTP/1.1 y HTTP/2, Manejo de Cabeceras, Caché y Tolerancia a Fallos

---

## 1. Cabeceras Obligatorias y Negociación de Red

### 1.1. Supresión Obligatoria de Compresión (`Accept-Encoding: identity`)
En descargas aceleradas por rangos de bytes, **todas las peticiones HTTP DEBEN enviar explícitamente**:
```http
Accept-Encoding: identity
```
* **Justificación Crítica:** Si un servidor comprime la respuesta (`Content-Encoding: gzip` o `br`), los desplazamientos de bytes de la cabecera `Range` corresponden al flujo comprimido en tránsito, no al archivo descomprimido en disco. Intentar escribir chunks comprimidos con `pwrite64` corrompe de manera irreversible el archivo final. Si un servidor responde con `Content-Encoding` distinto de `identity`, el motor debe abortar el modo multi-conexión y descargar secuencialmente en un pipeline de descompresión al vuelo.

---

### 1.2. Política de Redirecciones y Seguridad SSRF
1. **Seguimiento de Redirecciones:**
   * El cliente seguirá códigos `301`, `302`, `303`, `307` y `308` hasta un límite estricto de **10 saltos**.
   * Si se supera el límite, la tarea transiciona a estado `failed` con código `ERR_TOO_MANY_REDIRECTS`.
   * La validación SSRF se repite en **cada salto**, después de resolver el host de destino; no alcanza con validar la URL original.
2. **Preservación y Descarte de Cabeceras:**
   * En redirecciones entre dominios distintos (*cross-origin*), las cabeceras sensibles `Authorization` y `Cookie` **DEBEN eliminarse automáticamente** a menos que el usuario las haya configurado explícitamente para el nuevo dominio.
   * La cabecera `Referer` se actualizará a la URL previa.
3. **Protección SSRF (Server-Side Request Forgery):**
   * Antes de abrir un socket TCP/TLS, la dirección IP resuelta por DNS se valida contra rangos privados y locales:
     * `127.0.0.0/8`, `::1` (Loopback)
     * `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `fc00::/7` (Redes privadas)
     * `169.254.0.0/16`, `fe80::/10` (Link-local)
   * Las descargas dirigidas a IPs privadas son bloqueadas por defecto (`ERR_BLOCKED_PRIVATE_IP`), a menos que la opción `"allow_local_downloads": true` esté activa.

---

### 1.3. Sanitización de `Content-Disposition` y Prevención de Path Traversal
El nombre de archivo extraído de la cabecera `Content-Disposition: attachment; filename="..."` o `filename*="UTF-8''..."` (RFC 6266) es potencialmente malicioso.
1. Se procesa primero `filename*` (RFC 5987) por tener prioridad sobre `filename`.
2. **Reglas de Sanitización Obligatorias:**
   * Extraer únicamente el nombre base (`basename`), eliminando cualquier prefijo de ruta (`/`, `\`, `../`, `..\\`).
   * Eliminar caracteres de control ASCII (`0x00` a `0x1F`, `0x7F`).
   * Reemplazar caracteres prohibidos en sistemas Linux por guiones bajos `_`: `\0` y `/`.
   * Truncar la longitud del nombre a un máximo de **255 bytes** respetando límites de UTF-8.
   * Si el nombre sanitizado resulta vacío o consta solo de puntos (`.` o `..`), asignar como fallback: `download_{task_id}.bin`.

---

### 1.4. TLS, proxy y resolución
* TLS se valida con el almacén del sistema/rustls; `danger_accept_invalid_certs` y equivalentes están
  prohibidos. HTTP/2 es opcional; si el servidor negocia HTTP/1.1, el contrato de rangos no cambia.
* La resolución DNS se hace una vez por conexión y se revalida después de cada redirección para evitar
  DNS rebinding y SSRF. El daemon no acepta `file:`, `data:`, `ftp:` ni esquemas fuera de `http(s)`.
* El proxy se toma de `settings.proxy_mode` (`system`, `none`, `manual`); el modo manual usa
  `proxy_url` y credenciales del llavero. Las credenciales de proxy nunca se escriben en logs ni en
  SQLite.
* Un `407 Proxy Authentication Required` se trata como `ERR_PROXY_AUTH` y pausa la tarea para que el
  usuario actualice el secreto; no se reintenta en bucle.

## 2. Validadores de Caché y Reanudación Segura

Para garantizar que un archivo pausado no se reanude sobre una versión que ha cambiado en el servidor, el motor almacena y valida los siguientes campos:

### 2.1. Almacenamiento de Validadores en la Fase de Sondeo
Durante el sondeo inicial, el daemon almacena en base de datos:
* `etag`: Valor exacto de la cabecera `ETag` (ej. `"686897696a7c876b7e"`).
* `last_modified`: Valor de la cabecera `Last-Modified` (ej. `Wed, 21 Oct 2026 07:28:00 GMT`).

### 2.2. Uso de `If-Range` en Conexiones de Reanudación
En cada petición de segmento tras una pausa o reconexión se envía **un** validador fuerte, en este orden:
`ETag` si existe; si no, `Last-Modified`. Si no hay ninguno, se repite el sondeo y no se reanuda a ciegas.
```http
GET /archivo.iso HTTP/1.1
Host: mirror.ubuntu.com
Range: bytes=10485760-20971519
If-Range: "686897696a7c876b7e"
```
* **Evaluación de la Respuesta:**
  * **`206 Partial Content`**: El validador coincide. El archivo en el servidor es idéntico. El worker continúa escribiendo en su desplazamiento.
  * **`200 OK`**: El archivo en el servidor fue modificado (el ETag cambió o expiró).
    * **Acción Obligatoria:** Abortar inmediatamente todos los workers.
    * Truncar el archivo local en disco a 0 bytes.
    * Reiniciar el estado de todos los segmentos a cero.
    * Notificar a la GUI el evento `download_restarted_due_to_stale_cache`.

---

## 3. Matriz de Códigos de Estado HTTP y Comportamiento

| Código HTTP | Significado | Acción del Motor de Segmenta |
| :--- | :--- | :--- |
| **`200 OK` (en Sondeo)** | Servidor ignora `Range` | Conmuta a modo **Single-Stream** (descarga lineal secuencial). |
| **`200 OK` (durante Reanudación)** | Archivo modificado en origen | Reinicia la descarga desde byte 0 (ver sección 2.2). |
| **`206 Partial Content`** | Rango aceptado | Operación nominal multi-conexión. |
| **`416 Range Not Satisfiable`** | Desplazamiento fuera de rango | Validar si `next_offset == total_bytes`. Si coincide, marcar segmento como completado. Si no, error `ERR_RANGE_OUT_OF_BOUNDS`. |
| **`429 Too Many Requests`** | Servidor saturado / Rate limit | Suspender worker temporalmente. Extraer `Retry-After` (segundos o fecha HTTP); si la fecha es inválida o pasada, usar 1 s. Sin cabecera, backoff exponencial (mínimo 5 s, máximo 60 s) con jitter. Reducir conexiones activas para ese dominio. |
| **`500, 502, 503, 504`** | Errores transitorios de servidor | Estado `retrying`. Aplicar reintentos con backoff exponencial y jitter (ver sección 4). |
| **`401 Unauthorized / 403 Forbidden`** | Credenciales inválidas/expiradas | Estado `paused` con error `ERR_AUTH_REQUIRED`. Solicitar actualización de cookies/credenciales al usuario o extensión. No reintentar en bucle. |
| **`404 Not Found / 410 Gone`** | Recurso inexistente | Estado `failed` definitivo (`ERR_RESOURCE_GONE`). No reintentar. |

---

## 4. Política de Reintentos, Backoff y Clasificación de Errores

### 4.1. Taxonomía de Errores

```
                                  [ Error en Worker ]
                                           |
                  +------------------------+------------------------+
                  |                                                 |
         [ Error Recuperable ]                            [ Error Terminal ]
         - ConnectionReset / BrokenPipe                   - 404 Not Found / 410 Gone
         - DNS Timeout / TCP Timeout                      - 401/403 (Sin credenciales)
         - HTTP 429, 500, 502, 503, 504                   - Espacio en disco lleno (ENOSPC)
         - Conexión caída abruptamente                    - Permisos denegados (EACCES)
                  |                                                 |
                  v                                                 v
        [ Backoff Exponencial ]                             [ Estado FAILED ]
```

### 4.3. Códigos de error del motor

Los errores que nacen en el motor no siempre tienen un HTTP status detrás. Este es el mapeo
normativo entre el identificador que ven los usuarios (`downloads.error_message`), el código
numérico que viaja en JSON-RPC (`error.code` / `data.code`) y se guarda en
`downloads.error_code`, y la acción que toma el daemon.

| Código | Identificador | Origen | Acción |
| :--- | :--- | :--- | :--- |
| `1004` | `ERR_INVALID_URL` | esquema no http(s), host ausente, DNS sin resultados | `failed` |
| `1005` | `ERR_DESTINATION_UNWRITABLE` | `EACCES`/`EPERM` en la carpeta destino | `paused` |
| `1006` | `ERR_DISK_FULL` | `posix_fallocate`/`fstatvfs` sin espacio | `paused` |
| `1007` | `ERR_BLOCKED_PRIVATE_IP` | SSRF: la IP resuelta es privada y `allow_local_downloads` es falso | `failed` |
| `1008` | `ERR_CHECKSUM_MISMATCH` | SHA-256 final distinto del esperado | `failed` |
| `1021` | `ERR_TOO_MANY_REDIRECTS` | más de 10 saltos | `failed` |
| `1022` | `ERR_RESOURCE_GONE` | `404`/`410` | `failed` |
| `1023` | `ERR_AUTH_REQUIRED` | `401`/`403` | `paused` |
| `1024` | `ERR_PROXY_AUTH` | `407` | `paused` |
| `1025` | `ERR_RANGE_OUT_OF_BOUNDS` | `416` en un rango que no es el final del archivo | `failed` |
| `1026` | `ERR_SIZE_MISMATCH` | los bytes en disco no coinciden con `total_bytes` | `failed` |
| `1027` | `ERR_TRUNCATED_RESPONSE` | el cuerpo terminó antes del `Content-Length` | reintento |
| `1028` | `ERR_UNSUPPORTED` | respuesta comprimida o forma de respuesta no utilizable | `failed` |
| `1029` | `ERR_NETWORK` | reset, timeout, TLS, DNS intermitente | reintento |
| `1030` | `ERR_CANCELLED` | pausa o cancelación pedida por el usuario | `paused` |
| `1031` | `ERR_KEYRING_UNAVAILABLE` | no se pudo leer el secreto del llavero | `paused` |
| `1032` | `ERR_MALFORMED_HEADER` | `Content-Range` o cabecera de rango inválida | `failed` |

### 4.2. Fórmula de Backoff con Jitter
Para evitar el problema del rebaño (*thundering herd*):
$$\text{espera}_n = \min\left(\text{BACKOFF\_MAX}, \text{BACKOFF\_BASE} \times 2^{n}\right) \pm \text{jitter}$$
* Constantes:
  * $\text{BACKOFF\_BASE} = 1.0\text{ segundos}$
  * $\text{BACKOFF\_MAX} = 60.0\text{ segundos}$
  * $\text{MAX\_RETRIES} = 5$ (después de 5 intentos fallidos consecutivos en todos los workers, pausar descarga y alertar al usuario).
  * $\text{jitter} = \text{rand}(0.0, 0.5 \times \text{espera}_n)$

---

## 5. Manejo de Descargas Especiales

### 5.1. Descargas sin `Content-Length` o `Transfer-Encoding: chunked`
* Si el servidor no envía `Content-Length` ni cabecera `Content-Range`:
  1. No se invoca `posix_fallocate` (tamaño desconocido).
  2. Se abre el descriptor de archivo destino en modo append/escritura secuencial.
  3. En la base de datos se registra `total_bytes = NULL`.
  4. La descarga se procesa con un **único worker**.
  5. Se leen bytes continuamente hasta recibir `EOF` de la conexión.
  6. Al cerrar el socket, el tamaño final del archivo en disco se lee mediante `fstat(fd)` y se actualiza `total_bytes` en base de datos.
  7. **Semántica de Pausa en Single-Stream:**
     * Si el servidor **no soporta rangos**, la acción "Pausar" aborta la descarga con advertencia: *"Este servidor no permite reanudar; si pausa la descarga tendrá que reiniciar desde el principio"*.

### 5.2. Verificación de Integridad de Archivo Final
1. Si la petición incluye un hash esperado (proporcionado en la solicitud JSON-RPC vía `"expected_checksum": {"algorithm": "sha256", "hash": "..."}` o extraído de cabeceras `Digest: sha-256=...`):
   * Al finalizar todos los segmentos, la tarea transiciona al estado intermedio `verifying`.
   * Se lee el archivo final en bloques de 1 MB calculando el digest SHA-256.
   * Si el hash calculado coincide: estado `completed`.
   * Si no coincide: estado `failed` con código `ERR_CHECKSUM_MISMATCH` (el archivo corrupto se renombra con sufijo `.corrupt` para evitar sobrescribir datos válidos).

---

## 6. Integridad de Tamaño y Cierre

Además del checksum opcional, el motor verifica que los bytes escritos coincidan con el tamaño declarado:

- Si `total_bytes` es conocido y `sum(segmentos_completos) != total_bytes`, la tarea falla con
  `ERR_SIZE_MISMATCH` y no se marca `completed`.
- Si un `206` declara un `Content-Range` cuyo `total` difiere del sondeo original, se trata como archivo
  remoto cambiado: se reinicia desde cero, nunca se mezclan generaciones.
- Si el `Content-Length` de una respuesta `200` no coincide con lo escrito hasta EOF, se falla con
  `ERR_TRUNCATED_RESPONSE`.
- El cierre sólo se considera exitoso después de `fsync`, verificación de tamaño/checksum y transición
  atómica a `completed` (ver 08).
