# OpenSpec 02: Motor de Descargas y Algoritmo de Segmentación Dinámica

> **Especificación:** OPENSPEC-SEGMENTA-002  
> **Versión:** 1.0.0  
> **Área:** Núcleo de Red, Planificación Concurrente y E/S de Archivos

---

## 1. Fases del Ciclo de Vida de una Descarga

Cada descarga procesada por el motor de Segmenta atraviesa una máquina de estados determinista:

```mermaid
stateDiagram-v2
    [*] --> Probing
    Probing --> SingleStream: Servidor NO soporta Ranges
    Probing --> Preallocating: Servidor soporta Ranges
    Preallocating --> Downloading: fallocate() exitoso
    Downloading --> Splitting: Un hilo termina y divide otro
    Splitting --> Downloading
    Downloading --> Completed: 100% descargado
    Downloading --> Paused: Pausa solicitada / Error recuperable
    Paused --> Downloading: Reanudar
    Downloading --> Failed: Error irrecuperable
    Completed --> [*]
```

---

## 2. Fase de Sondeo (*Probing*)

Antes de lanzar hilos concurrentes, el motor ejecuta una solicitud ligera de sondeo:

```http
GET /recurso.zip HTTP/1.1
Host: cdn.servidor.com
User-Agent: Mozilla/5.0 (X11; Ubuntu; Linux x86_64) ...
Range: bytes=0-1
Connection: keep-alive
```

### Reglas de Evaluación de Respuesta:
1. **Soporte de Rangos Confirmado (`206 Partial Content`)**:
   * Si la cabecera `Content-Range` está presente con formato `bytes 0-1/TOTAL_BYTES`:
     $$\text{total\_size} = \text{TOTAL\_BYTES}$$
     Se activa el modo de **Segmentación Dinámica Concurrente** con $N$ conexiones (predeterminado: 16).
2. **Servidor Estándar sin Soporte de Rangos (`200 OK`)**:
   * El servidor ignora la cabecera `Range`. El archivo no se puede particionar.
   * Se conmuta a **Single-Stream** (descarga secuencial tradicional con un solo hilo).
3. **Contenido Dinámico sin Tamaño (`Transfer-Encoding: chunked`)**:
   * No existe `Content-Length`. Se descarga en modo *streaming pipe* directamente a disco hasta el cierre de la conexión (`EOF`).

---

## 3. Algoritmo de Segmentación Dinámica en Caliente (*Dynamic Split*)

Este algoritmo es el pilar central de rendimiento de Segmenta y resuelve el problema de la desaceleración por conexiones estranguladas o hilos ociosos.

### 3.1. Partición Inicial
Dado un tamaño total $S$ y un número de conexiones objetivo $N$:
* Si $S < 5 \text{ MB}$, usar $N = 1$.
* Si $5 \text{ MB} \le S < 50 \text{ MB}$, usar $N = 4$.
* Si $S \ge 50 \text{ MB}$, usar $N = 16$ (o el valor configurado por el usuario, hasta 32).

Se calcula el tamaño base de cada segmento inicial:
$$\text{chunk\_size} = \left\lfloor \frac{S}{N} \right\rfloor$$

Para cada segmento $i \in [0, N-1]$:
$$\text{start\_offset}_i = i \times \text{chunk\_size}$$
$$\text{end\_offset}_i = \begin{cases} (i + 1) \times \text{chunk\_size} - 1 & \text{si } i < N - 1 \\ S - 1 & \text{si } i = N - 1 \end{cases}$$
$$\text{current\_offset}_i = \text{start\_offset}_i$$

---

### 3.2. Lógica de Fisión Dinámica (*Hot-Splitting*)

Cada hilo de trabajo (*worker*) reporta continuamente su `current_offset`.

1. **Evento de Disparo**: El worker $W_k$ finaliza la descarga de su segmento asignado $S_k$ ($\text{current\_offset}_k = \text{end\_offset}_k$).
2. **Búsqueda de Donante**:
   * El planificador itera sobre todos los segmentos activos $\{S_j\}$ que están siendo descargados por otros workers.
   * Calcula los bytes restantes por descargar de cada uno:
     $$\text{restante}_j = \text{end\_offset}_j - \text{current\_offset}_j$$
   * Selecciona el segmento con el mayor número de bytes restantes:
     $$S_{\text{max}} = \arg\max_j (\text{restante}_j)$$
3. **Umbral de Fisión**:
   * Definir constante $\text{MIN\_SPLIT\_SIZE} = 2 \text{ MB}$ ($2{,}097{,}152$ bytes).
   * **Condición:** Si $\text{restante}_{\text{max}} < \text{MIN\_SPLIT\_SIZE}$, **no dividir**. El worker $W_k$ entra en reposo (*idle*), ya que el coste de handshake TLS y latencia de conexión supera el beneficio de dividir una porción pequeña.
4. **Ejecución de la División**:
   * Si $\text{restante}_{\text{max}} \ge \text{MIN\_SPLIT\_SIZE}$:
     $$\text{punto\_medio} = \text{current\_offset}_{\text{max}} + \left\lfloor \frac{\text{restante}_{\text{max}}}{2} \right\rfloor$$
     * El segmento original $S_{\text{max}}$ reduce su límite superior en caliente:
       $$\text{end\_offset}_{\text{max}} = \text{punto\_medio}$$
     * Se crea un nuevo segmento $S_{\text{nuevo}}$ y se asigna al worker ocioso $W_k$:
       $$\text{start\_offset}_{\text{nuevo}} = \text{punto\_medio} + 1$$
       $$\text{current\_offset}_{\text{nuevo}} = \text{punto\_medio} + 1$$
       $$\text{end\_offset}_{\text{nuevo}} = \text{fin\_antiguo}_{\text{max}}$$
5. **Reconexión Inmediata**:
   * El worker $W_k$ reutiliza el pool de conexiones o abre un nuevo socket TCP y envía:
     ```http
     GET /recurso.zip HTTP/1.1
     Host: cdn.servidor.com
     Range: bytes=[punto_medio + 1]-[fin_antiguo]
     ```

---

## 4. Subsistema de E/S de Disco (Sin Ensamblado)

A diferencia de Windows, donde cada conexión escribe en un archivo temporal separado y luego se copian secuencialmente, **Segmenta escribe directamente en el archivo destino**:

### 4.1. Preasignación con `posix_fallocate`
Al conocer el `total_size`:
```c
int fd = open(final_path, O_RDWR | O_CREAT | O_CLOEXEC, 0644);
if (fd < 0) {
    /* Manejar error de permisos o ruta */
}

// Reserva física garantizada de bloques en el sistema de archivos
int ret = posix_fallocate(fd, 0, total_size);
if (ret != 0) {
    // ENOSPC: Espacio insuficiente en disco. Abortar antes de descargar.
}
```
* **Ventaja:** Elimina la fragmentación del archivo en ext4/btrfs/xfs y asegura de antemano que el disco no se llenará a mitad del proceso.

### 4.2. Escrituras Atómicas Desplazadas con `pwrite64`
Cada worker mantiene su propio búfer de lectura en memoria (ej. 64 KB o 128 KB). Al recibir un chunk desde el socket de red:

```c
ssize_t written = pwrite64(fd, buffer, bytes_received, segment->current_offset);
if (written > 0) {
    segment->current_offset += written;
    atomic_fetch_add(&global_downloaded_bytes, written);
}
```

* **Ventaja:** `pwrite64` no utiliza ni altera el puntero interno del descriptor (`lseek`). Cientos de escrituras concurrentes pueden ejecutarse simultáneamente sin condiciones de carrera y sin bloqueos de mutex a nivel de archivo.
* **Fin de Descarga Instantáneo:** En el instante en que el último worker escribe su último bloque, se llama a `fsync(fd)` y `close(fd)`. **El archivo está listo al 100% en 0 segundos.**

---

## 5. Control de Flujo y Estrangulamiento de Velocidad

Para respetar la opción de "Límite de Velocidad Global":
* Se implementa el algoritmo **Token Bucket**:
  * Un cubo (*bucket*) recibe tokens a una tasa de $R$ bytes/segundo.
  * Antes de que cualquier worker lea datos de su socket, debe solicitar $B$ tokens del cubo global.
  * Si el cubo está vacío, el worker espera asíncronamente en el runtime (`tokio::time::sleep` en Rust o `time.Sleep` en Go) hasta la reposición de tokens.
