# Análisis Técnico de Arquitectura: Motores de Descarga Multi-Conexión

> **Documento:** RES-001  
> **Estado:** Completo / Aprobado  
> **Objetivo:** Documentar los principios de ingeniería interna, protocolos y estructuras de datos observados en gestores de descarga multi-conexión para fundamentar la arquitectura de **Segmenta**.

---

## 1. Alcance y Metodología de Investigación

La investigación se centró en estudiar los componentes estructurales de los motores de descarga acelerada tradicionales en entornos Windows:
1. El ejecutable del gestor central (núcleo y UI clásica)
2. El proceso puente de mensajería nativa para navegadores (*Native Messaging Host*)
3. El módulo de sincronización local y servidor WebSocket interno
4. El parser de manifiestos multimedia para streaming (HLS / DASH)
5. La extensión WebExtension de captura profunda (Manifest V3)

---

## 2. Análisis Estructural de Componentes

### 2.1. El Motor Central de Descargas
* **Formato Binario y Runtime:** Binario PE32 (x86, 32-bit), Subsistema Windows GUI.
* **Manejo de Red:**
  * No depende de APIs HTTP de alto nivel del sistema operativo. Maneja directamente sockets TCP asíncronos y resuelve funciones de red dinámicamente en tiempo de ejecución.
* **Capa Criptográfica y TLS:**
  * Uso de interfaces nativas TLS del sistema operativo para negociación de certificados y TLS 1.3, con rutinas de respaldo basadas en OpenSSL para servidores con configuraciones no estándar.
* **Framework y GUI:** Enlace estático con primitivas clásicas de ventanas y diálogos de control de estado.

---

### 2.2. Proceso Puente de Mensajería Nativa (*Native Messaging Host*)
* **Formato:** Proceso nativo desacoplado ejecutado bajo demanda por el navegador cuando una extensión solicita comunicación externa (`chrome.runtime.connectNative`).
* **Flujo de E/S:**
  * **Hilo de Entrada (*Input Thread*):** Lee paquetes de `stdin` formateados con el protocolo estándar de Native Messaging de Chromium (encabezado little-endian de 32 bits indicando la longitud $L$, seguido de $L$ bytes de carga JSON/binaria) y los redirige inmediatamente a un canal IPC local (Pipes / Sockets).
  * **Hilo de Salida (*Output Thread*):** Lee respuestas del motor central y las emite por `stdout` hacia el navegador.

---

### 2.3. Módulo de Sincronización Local de Red
* Aloja un servidor WebSocket local en la interfaz de bucle invertido (`127.0.0.1`):
  * Negocia la conexión HTTP inicial:
    ```http
    HTTP/1.1 101 Switching Protocols
    Upgrade: websocket
    Connection: Upgrade
    Sec-WebSocket-Protocol: plugin.v3.download-manager
    ```
  * Permite que la extensión del navegador mantenga un canal persistente de baja latencia con el núcleo sin tener que crear y destruir repetidamente procesos hijos.

---

### 2.4. Módulo de Procesamiento de Streaming Adaptativo
* Parser de manifiestos multimedia para reconstrucción de video streaming bajo demanda:
  * **Manifiestos MPEG-DASH:** Parseo de elementos XML: `SegmentTimeline`, `SegmentURL`, `SegmentTemplate`, `SegmentList`, `SegmentBase`, `Initialization`.
  * **Manifiestos HLS (`.m3u8`):** Parseo de listas maestras y de variantes (`EXT-X-STREAM-INF`, `EXT-X-MEDIA`, `EXTINF`).
  * Permite descargar los fragmentos (`.ts`, `.m4s`) en paralelo y ensamblarlos en un único contenedor (`.mp4`, `.mkv`).

---

## 3. Arquitectura de la Extensión del Navegador

Al analizar las extensiones de captura profunda en navegadores modernos:
* **Hooking en Página (`document.js`):** Para no perder peticiones AJAX o llamadas a API de video creadas dinámicamente:
  * Se inyecta en el contexto de la página y supervisa llamadas a `window.fetch`, `XMLHttpRequest.prototype.open/send` y `HTMLMediaElement.prototype.play`.
  * Captura las URLs de los manifiestos (`.mpd`, `.m3u8`) y emite eventos hacia el script de contenido.
* **Paquete de Contexto de Descarga:**
  * **URL resuelta**: URL final tras cualquier redirección.
  * **Cookies de Sesión**: Extraídas mediante la API del navegador para permitir descargas tras muros de login.
  * **Cabeceras HTTP**: `User-Agent`, `Referer`, `Authorization`.
  * **Metadatos sugeridos**: Nombres extraídos de `Content-Disposition`.

---

## 4. El Algoritmo de Segmentación Dinámica (*Dynamic Segmentation*)

La clave técnica para acelerar descargas frente a herramientas lineales reside en la concurrencia dinámica:

### 4.1. Verificación Inicial
1. Petición inicial con rango mínimo (`Range: bytes=0-1`):
   * Si el servidor responde `206 Partial Content` y expone `Content-Range: bytes 0-1/TOTAL_BYTES`, se activa la descarga multi-hilo con $N$ conexiones concurrentes.
   * Si responde `200 OK` ignorando el rango, se conmuta a modo lineal de conexión única.

### 4.2. División en Caliente de Bloques (*Hot-Splitting*)
* A diferencia de gestores estáticos que dividen el archivo en bloques rígidos que dejan conexiones ociosas al terminar antes:
* Cuando un hilo $A$ termina su segmento, el planificador (*scheduler*) examina los segmentos de todos los hilos activos y localiza al hilo $B$ con la mayor cantidad de bytes restantes.
* El planificador parte el rango pendiente de $B$ por la mitad:
  * El hilo $B$ ajusta su límite superior al punto medio.
  * El hilo libre $A$ abre una nueva conexión con:
    ```http
    GET /file.ext HTTP/1.1
    Host: example.com
    Range: bytes=OFFSET_MEDIO-FIN_ORIGINAL
    ```
* Este proceso se repite continuamente hasta completar la descarga.

---

## 5. Cuello de Botella Tradicional y la Solución en Linux

### El Problema de Diseño Tradicional:
Muchos gestores clásicos almacenan cada segmento en un archivo temporal separado en disco.
* Con 16 o 32 conexiones, crean 16 o 32 archivos pequeños independientes.
* Al finalizar el 100% de la transferencia, entran en una fase de **Ensamblado** (*Assembling*): leen secuencialmente cada archivo de segmento y los concatenan en el destino final.
* En archivos de gran tamaño (10 GB a 50 GB), esta fase puede tardar minutos, congela el sistema y desgasta unidades de almacenamiento.

### La Solución en Linux (Segmenta):
Bajo el kernel Linux y sistemas de archivos POSIX modernos (ext4, btrfs, xfs, ZFS), este problema se erradica completamente mediante dos llamadas al sistema:
1. **`posix_fallocate(fd, 0, total_size)`**:
   * Preasigna de manera instantánea el espacio total del archivo en los metadatos del filesystem sin necesidad de escribir ceros.
   * Garantiza que no habrá errores de disco lleno y previene la fragmentación.
2. **`pwrite64(fd, buf, count, offset)`**:
   * Escribe datos atómicamente en cualquier desplazamiento (*offset*) arbitrario sin alterar el puntero de archivo del descriptor ni bloquear a otros hilos.
   * **Resultado:** Todas las conexiones concurrentes escriben directamente en el archivo final. Cuando el último segmento llega a cero bytes pendientes, **el archivo ya está 100% completo y listo para usarse inmediatamente, con cero tiempo de ensamblado**.
