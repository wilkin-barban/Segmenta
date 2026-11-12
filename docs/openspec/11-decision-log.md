# OpenSpec 11: Decisiones de v1 y Definición de Listo

> **Especificación:** OPENSPEC-SEGMENTA-011
> **Versión:** 2.0.0 (0.2.0 **en preparación**)
> **Área:** Alcance cerrado, precedencia documental, decisiones de ingeniería y criterios para empezar cada fase
>
> §8 registra las decisiones de 0.2.0. Marcadas «(0.2, en preparación)», son el contrato acordado y no
> comportamiento entregado hasta que Q1 cierre con el QA real y exista el tag `v0.2.0`.

---

## 1. Precedencia documental

Cuando dos documentos se contradigan, se aplica este orden:

1. `06-http-contract.md` para HTTP, validadores, reintentos e integridad.
2. `07-ipc-contract.md` para JSON-RPC, métodos, errores, eventos, socket y ciclo de vida.
3. `08-persistence-recovery.md` para el DDL, `next_offset`, hot-split seguro, secretos y recuperación.
4. `09-extension-security.md` para manifiestos, IDs, inyección, Native Messaging y streaming.
5. `10-packaging-testing.md` para workspace, `.deb`, CI y benchmarks.
6. `12-queue-scheduler-postprocess-auth.md` para scheduler, `max_active`, hook argv, ClamAV/cuarentena
   y site logins (0.2, en preparación).
7. `01`–`05` son la visión y el roadmap; si resumen algo, mandan los contratos anteriores.
8. `research/legacy-engine-analysis.md` es contexto histórico, no una especificación de implementación.

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
- Empaquetado `.deb` para Ubuntu 26.04, unidad systemd de usuario, `.desktop`, AppStream y
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

Cada punto requiere un OpenSpec nuevo antes de implementarse; no son decisiones implícitas. El estado
vigente de esta deuda, agrupado por causa, está en §9.

---

## 8. Decisiones de 0.2 (0.2, en preparación)

Cada fila es una decisión cerrada con su razón de una línea; el contrato operativo vive en
`12-queue-scheduler-postprocess-auth.md` y en la sección citada.

| Tema | Decisión | Razón (una línea) | Contrato |
| --- | --- | --- | --- |
| Scheduler | Loop nativo `tokio` dentro del daemon, tick de 30 s, `queue.schedule` como fuente de verdad | Un scheduler en el daemon no depende de la GUI abierta y un solo lugar aplica la política | 12 §2 |
| Límite de cola | `max_active` real aplicado por el daemon antes de `start_task` | La GUI no puede ser la que garantice el límite sin carreras | 12 §3 |
| Acción al completar | `none`, `suspend`, `power_off`, `exit`, sólo cuando la cola queda vacía | Evita suspender el equipo con descargas aún pendientes | 12 §2 |
| Post-proceso | Hook argv (`Vec<String>`) con `%file%`, `%folder%`, `%name%`, timeout y salida acotada | Sin shell no hay inyección por nombre de archivo | 12 §4 |
| Antivirus | ClamAV integrado y **opcional** (`antivirus_enabled=false` por defecto), `clamdscan` antes de `clamscan` | Embeberlo obligaría a arrastrar firmas y licencias que el `.deb` no puede mantener | 12 §5 |
| Infección | Cuarentena y tarea `failed` con `ERR_ANTIVIRUS_FOUND` | Un archivo infectado nunca debe quedar en Descargas ni figurar como completado | 12 §5 |
| Antivirus ausente | `ERR_ANTIVIRUS_UNAVAILABLE` en vez de `completed` | Declarar limpio lo que no se escaneó sería mentir al usuario | 12 §5 |
| Credenciales | HTTP Basic + GNOME Keyring por dominio, `site_login.list`/`set`/`remove`, password nunca devuelta | Basic sobre TLS cubre el caso real; guardar el secreto en SQLite lo descarta | 12 §6 |
| Digest | Fuera de alcance (limitación documentada, sin promesa) | Digest obliga a leer el challenge y añade superficie sin demanda comprobada | 12 §6 |
| Categorías | Sin categorías automáticas; se conserva sólo la lectura/migración legacy del ajuste | Clasificar por extensión produce carpetas sorpresa y no aporta al flujo de descarga | 04, 07 |
| Sniffer | Sólo YouTube; el resto entra por captura genérica opt-in por sitio | Mantener adaptadores por sitio implica perseguir cambios ajenos sin fin | 09 §8 |
| Extensión | Instalación manual asistida por GUI; sin Chrome Web Store ni AMO | Las tiendas exigen firma, revisión y no existen en el plazo de 0.2 | 09 §7 |
| Empaquetado | `.deb` 0.2.0 con los dos ZIP y 22 rutas validadas; ClamAV opcional fuera de `Depends` | El paquete debe instalar lo que la puerta verifica, ni más ni menos | 10 §2 |
| Versión | 0.2.0 en preparación; tag `v0.2.0` sólo con Q1 verde | El tag es la afirmación de que el QA real pasó, no el comienzo del trabajo | 10 §6, ODD |

Origen: plan aprobado `segmenta-0.2.md` y `odd/tasks/segmenta-0.2.md` (decisiones cerradas). Ninguna
fila de §8 autoriza código nuevo por sí sola: si una tarea necesita un método, campo o evento que no
está en `12`, primero se agrega al OpenSpec.

---

## 9. Deuda explícita restante al cierre de 0.2

Límites de plataforma (documentados como límites, nunca como bugs abiertos ni promesas):

- **Scheduler sin daemon**: una ventana horaria no se ejecuta si el daemon no está activo; la
  activación por socket despierta el proceso, no adelanta el reloj.
- **Chrome estable**: no permite instalar la extensión automáticamente sin tienda o política de
  empresa; la GUI sólo prepara y abre. Chrome for Testing es el entorno de QA.
- **Firefox snap y host nativo**: el sandbox no puede ejecutar `/usr/bin/segmenta-host`; no se intenta
  burlar.
- **Firefox sin firma**: la carga desde `about:debugging` es temporal y se pierde al reiniciar.
- **Cold-start de captura**: si el navegador entrega el evento antes de que exista el worker, la
  reconciliación con `downloads.search` reduce la carrera pero no la elimina.
- **Basic sin Digest**: no cubre servidores que sólo ofrecen Digest; no hay fallback silencioso.
- **ClamAV externo**: la calidad depende de firmas actualizadas que el `.deb` no gestiona.
- **Suspender/apagar**: requieren permisos del sistema y no se prueban de forma destructiva en CI ni
  en el QA de 0.2.

Trabajo pendiente que no entra en 0.2 y exige OpenSpec nuevo antes de implementarse:

- Flatpak/Snap/AppImage y Native Messaging en sandbox; tiendas y firma de extensiones.
- DRM, HLS cifrado, live con ventanas móviles, subtítulos y multi-audio.
- Digest y otros esquemas de autenticación; OAuth o gestores de contraseñas externos.
- Adaptadores de sniffer fuera de YouTube.
- Telemetría, actualizaciones automáticas, perfiles de usuario y sincronización entre dispositivos.
- Migración de datos desde otros gestores de descarga.
- Rotación de claves de firma y publicación automatizada de artefactos.
