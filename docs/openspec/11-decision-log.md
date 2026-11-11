# OpenSpec 11: Decisiones de v1 y Definición de Listo

> **Especificación:** OPENSPEC-SEGMENTA-011
> **Versión:** 1.0.0
> **Área:** Alcance cerrado, precedencia documental, decisiones de ingeniería y criterios para empezar cada fase

---

## 1. Precedencia documental

Cuando dos documentos se contradigan, se aplica este orden:

1. `06-http-contract.md` para HTTP, validadores, reintentos e integridad.
2. `07-ipc-contract.md` para JSON-RPC, métodos, errores, eventos, socket y ciclo de vida.
3. `08-persistence-recovery.md` para el DDL, `next_offset`, hot-split seguro, secretos y recuperación.
4. `09-extension-security.md` para manifiestos, IDs, inyección, Native Messaging y streaming.
5. `10-packaging-testing.md` para workspace, `.deb`, CI y benchmarks.
6. `01`–`05` son la visión y el roadmap; si resumen algo, mandan los contratos anteriores.
7. `research/legacy-engine-analysis.md` es contexto histórico, no una especificación de implementación.

El DDL de `08` es el único esquema normativo. El bloque SQL de `01` fue retirado a propósito para que no
existan dos versiones del mismo esquema.

---

## 2. Alcance cerrado de v1

### Incluido

- Motor HTTP/HTTPS con rangos, segmentación inicial y hot-split con handoff cooperativo.
- Descargas directas, single-stream, `chunked`, reanudación validada por `ETag`/`Last-Modified`.
- Daemon por usuario, GUI GTK4/Libadwaita, CLI y host de Native Messaging.
- SQLite WAL con migraciones versionadas, secretos en GNOME Keyring/cookies por tarea.
- Extensión MV3 para Chrome/Chromium/Brave/Edge y Firefox, con manifiestos separados.
- HLS/DASH **públicos y sin cifrar**: VOD, fMP4/TS, init segment y audio/video separados.
- Empaquetado `.deb` para Ubuntu 26.10, unidad systemd de usuario, `.desktop`, AppStream y
  manifiestos nativos de sistema.
- Límite global de velocidad, colas básicas, programador horario y post-procesamiento sin shell.

### Fuera de v1

- DRM (Widevine, FairPlay, PlayReady), cifrado HLS AES-128/SAMPLE-AES y cualquier contenido protegido.
- HLS en vivo con ventanas móviles, subtítulos, multi-audio y cambios de pista a mitad de descarga.
- Flatpak, Snap y AppImage; el Native Messaging fuera del sandbox queda para un diseño posterior.
- Windows/macOS, sincronización entre dispositivos, cuentas remotas y navegador propio.
- Telemetría remota, actualizaciones automáticas y tiendas de extensiones en el primer corte.

Estos límites son funcionales, no “pendientes de investigación”: una implementación que los cruce sin
un OpenSpec nuevo está fuera de alcance.

---

## 3. Decisiones técnicas cerradas

| Tema | Decisión |
| --- | --- |
| Lenguaje | Rust, edición 2021, MSRV 1.80; workspace de 05/10 |
| Runtime | Tokio multi-thread; no se usa `async-std` |
| TLS | `rustls` por defecto; `native-tls` sólo como feature explícita |
| Persistencia | SQLite WAL con `user_version` y migraciones idempotentes |
| Secretos | GNOME Keyring/libsecret; si no está disponible, tarea `paused` con `ERR_KEYRING_UNAVAILABLE`; nunca SQLite en claro |
| IPC | JSON-RPC 2.0 NDJSON, Unix socket `0600`, `SO_PEERCRED`, versión 1 |
| Rangos | Intervalos cerrados `[start, end]`, `next_offset` = próximo byte a escribir |
| Hot-split | Handoff cooperativo con `SplitRequest`/`SplitAck`; el planificador no mueve `end_offset` solo |
| Escritura | `pwrite_all` con `EINTR`/partial writes; checkpoint de datos antes de SQLite |
| Finalización | `fsync`, verificación de tamaño/checksum y transición atómica; nunca publicar un parcial |
| Native Messaging | Manifiestos separados Chrome/Firefox, framing LE, límite 1 MiB, ID de desarrollo y producción separados |
| Streaming | Sólo HLS/DASH sin cifrar; DRM y AES-128 fuera de v1 |
| Empaquetado | `.deb` en v1; Flatpak/Snap/AppImage explícitamente fuera |
| Post-procesamiento | `shlex` + `Command`, sin `sh -c`; `%file%` como argumento |
| Licencia | GPL-3.0-or-later; `LICENSE` completo en el repo |

---

## 4. Contratos que un agente puede implementar sin preguntar

- **HTTP:** 06, incluyendo `Accept-Encoding: identity`, `If-Range`, redirecciones, SSRF, 429 y
  `ERR_SIZE_MISMATCH`.
- **IPC:** 07, incluyendo `system.hello`, catálogo de settings, eventos y tabla de errores.
- **Persistencia:** 08, incluyendo DDL, `download_headers`, `download_secrets`, colisiones y
  recuperación tras crash.
- **Extensión:** 09, incluyendo `world: MAIN`, `MessageChannel`, IDs y límites.
- **Build:** 10, incluyendo Cargo, `.deb`, CI y benchmarks.

Si una tarea necesita un método, campo o evento que no está en esos documentos, primero se agrega al
OpenSpec correspondiente; no se inventa dentro del código.

---

## 5. Definición de listo por fase

### Fase 1 — Motor

Listo para implementar cuando existan:

- `ProbeResult`, `Segment` y `pwrite_all` con tests unitarios de límites, 206/200, `chunked`, `EINTR` y
  `ENOSPC`.
- Un servidor de prueba con rangos, 429, ETag cambiado y corte abrupto.
- El DDL de 08 aplicado y una prueba de recuperación tras `SIGKILL`.

### Fase 2 — Daemon, IPC y host

- `system.hello`, `download.add/list/status/pause/resume/cancel`, eventos y errores de 07.
- Socket con `SO_PEERCRED`, permisos y límite de 1 MiB.
- Unidad systemd de 07 y host de 09 con framing little-endian y bucle continuo.

### Fase 3 — Extensión

- Manifiestos separados Chrome/Firefox, ID de desarrollo reproducible y `world: MAIN`.
- `download.add` sólo desde gesto del usuario; URL validada y SSRF revalidado en el daemon.
- Prueba de que una página no puede inyectar una descarga sin confirmación.

### Fase 4 — GUI

- Contrato IPC consumido sin acceso directo a SQLite ni HTTP.
- Estados vacío/cargando/error/pausado/cancelado y accesibilidad de 04 §7.
- Widget de segmentos probado con 1, 16 y 32 segmentos, sin caídas de frames.

### Fase 5 — Streaming

- Sólo manifiestos sin cifrar; init segment y orden de fragmentos.
- Prueba con un HLS VOD público y un DASH VOD público; rechazo explícito de DRM/AES-128.
- Dependencia de FFmpeg declarada y verificada al arrancar, no un binario embebido.

---

## 6. Criterios de aceptación globales

- Un archivo de 1 GB descargado con 16 conexiones coincide byte a byte y en SHA-256.
- Un corte de energía entre dos checkpoints reanuda sin huecos ni solapamientos.
- Un cambio de ETag reinicia la tarea y nunca mezcla generaciones.
- Una descarga con `Content-Encoding` distinto de `identity` no escribe bytes comprimidos en el archivo.
- Un cliente sin el mismo UID no puede hablar con el socket.
- Un archivo parcial nunca aparece como `completed`.
- El `.deb` instala, arranca por systemd y sobrevive a un reinicio de sesión.
- La extensión no puede iniciar una descarga sin gesto del usuario ni saltear la validación del daemon.

---

## 7. Deuda explícita (no bloquea Fase 1)

- Flatpak/Snap/AppImage y Native Messaging en sandbox.
- DRM, HLS cifrado, live, subtítulos y multi-audio.
- Rotación de claves de firma y publicación automatizada en tiendas.
- Telemetría, actualizaciones automáticas y perfiles de usuario.
- Migración de datos desde otros gestores.

Cada punto requiere un OpenSpec nuevo antes de implementarse; no son decisiones implícitas.
