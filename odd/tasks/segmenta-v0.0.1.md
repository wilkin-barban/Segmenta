# Segmenta 0.0.1 — primer release usable

Release `v0.0.1` de Segmenta: gestor de descargas directo, nativo, segmentado y reanudable, empaquetado
como `.deb` para Ubuntu 26.04 LTS. Documento de trabajo ODD; se lee hasta `**L6 — Evidencia de las puertas.** `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings` y `cargo test --workspace` en verde (151 tests, incluidos los 5 de
`cli_e2e`); `xvfb-run -a cargo test -p segmenta-gui` con las 5 pruebas de ventana reales; smoke
`SMOKE OK rows=1 segments=4` 5/5.

**L7 — Defectos encontrados al probar el `.deb` instalado.** (a) `segments.id` es el rowid global:
dos descargas que numeraban sus segmentos desde 0 colisionaban y todo checkpoint de la segunda
fallaba en silencio; ahora se inserta sin id explícito y hay test de regresión con dos tareas.
(b) `segmenta list` imprime ids de 8 caracteres pero los comandos exigían el UUID completo; ahora
se resuelve un prefijo único. (c) un `Completed` no persistía la tabla final de segmentos; (d)
`download_dir` no seguía a `settings`; (e) reanudar sin checkpoint chocaba con `EEXIST`.

**L8 — Verificación sobre el paquete instalado.** `.deb` `segmenta_0.0.1-1_amd64.deb` instalado con
`dpkg -i`, `segmenta.socket` habilitado, `segmenta hello` responde, GUI abre bajo `xvfb`, descarga de
10 MiB con 8 conexiones con SHA-256 idéntico, 32 MiB pausado a ~18 MiB + `SIGKILL` + reconexión +
reanudación con SHA-256 idéntico, y dos descargas simultáneas sin errores de checkpoint. `lintian`
no está instalado en el entorno: no se ejecutó (no se inventa el resultado).

## Log`.

## Specs

Cada spec es verificable y cita el requisito del usuario tal cual cuando aplica.

### S1 — Target Ubuntu 26.04 LTS
La documentación no menciona `26.10` en ningún archivo versionado.
Criterio: `grep -rn "26\.10" README.md docs/` no devuelve coincidencias; el entorno de build es
Ubuntu 26.04.1 LTS (GNOME 50, GTK 4.22, libadwaita 1.9).

### S2 — Workspace Rust
Cuatro crates: `crates/segmenta-core` (motor puro), `crates/segmenta-daemon` (daemon + IPC + SQLite),
`crates/segmenta-cli` (CLI de control), `crates/segmenta-gui` (GTK4 + Libadwaita).
`crates/segmenta-host` y `extension/` son stretch: no bloquean la release y no se crean si el resto no
está verde. Criterio: `cargo metadata --no-deps` lista exactamente los cuatro paquetes obligatorios.

### S3 — Sondeo HTTP/HTTPS
`Range: bytes=0-1` + `Accept-Encoding: identity` obligatorio. `206` con `Content-Range: bytes a-b/TOTAL`
⇒ `total_bytes = TOTAL` y modo segmentado. `200` que ignora `Range` ⇒ single-stream. Sin
`Content-Length` ni `Content-Range` o `Transfer-Encoding: chunked` ⇒ modo `chunked` a un solo worker.
`Content-Encoding` distinto de `identity` ⇒ se prohíbe multi-conexión. Se guardan `etag` y
`last_modified` del sondeo. Criterio: tests contra servidor local de rangos para 200, 206 y chunked.

### S4 — Segmentación inicial
`N` configurable 1–32 (default 16). `S < 5 MB` ⇒ 1 conexión; `5 MB ≤ S < 50 MB` ⇒ 4;
`S ≥ 50 MB` ⇒ `N` acotado a 32. `chunk = floor(S/N)`; el último segmento absorbe el resto hasta `S-1`.
Invariantes: `start` del primero = 0, `end` del último = `S-1`, `end_i + 1 == start_{i+1}`,
sin solapamientos ni huecos, `next_offset_i = start_i` al crearse. Criterio: property test de
cobertura exacta sobre tamaños de frontera (1, 5 MB-1, 5 MB, 50 MB-1, 50 MB, 1 GiB, no múltiplo de N).

### S5 — Hot-split (`SplitRequest`/`SplitAck`)
Umbral `MIN_SPLIT_SIZE = 2_097_152` bytes de bytes restantes; si el mayor donante queda por debajo, el
worker ocioso no divide. Handoff: el donante recibe `SplitRequest { donor_segment_id, target_midpoint }`,
responde `SplitAck { donor_segment_id, accepted, clamped_next_offset }` y aplica clamp de buffer para no
leer más allá de `target_midpoint`; sólo tras el `Ack` el nuevo segmento toma
`[clamped_next_offset + 1 .. original_end]` y el donante queda con `end = clamped_next_offset`.
Invariantes tras cualquier split: unión de rangos sin huecos ni solapes, y `downloaded_bytes` no
disminuye. Criterio: tests deterministas del planificador con donantes simulados, incluido ack tardío
con `next_offset` adelantado.

### S6 — E/S de disco sin ensamblado
`pwrite_all(fd, buf, offset)` repite hasta vaciar el buffer, tolera escrituras parciales y reintenta en
`EINTR`; nunca usa ni altera el puntero del descriptor. Preasignación con `posix_fallocate`; en
`ENOSPC` ⇒ `ERR_DISK_FULL` (1006) sin descargar; en `EOPNOTSUPP`/`ENOSYS` ⇒ `set_len` + comprobación de
espacio con `fstatvfs`, nunca fallo silencioso. Antes de confirmar un checkpoint se hace `fdatasync`.
El cierre hace `fsync` + `close` y transición atómica al path final. Criterio: tests de `pwrite_all`
con `EINTR` y escritura parcial inyectadas, de fallback de preasignación y de `ENOSPC` vía tmpfs del
tamaño justo cuando el entorno lo permita (si no, se simula el error de la capa syscall).

### S7 — Reanudación
`next_offset_i` es el próximo byte a escribir, nunca un byte ya escrito. Reanudar envía
`Range: bytes={next_offset}-{end_offset}` con `If-Range` de un único validador fuerte: `ETag`, y si no
existe `Last-Modified`. `206` continúa; `200` durante reanudación ⇒ abortar workers, truncar a 0,
resetear segmentos y reiniciar desde byte 0. Sin validadores ⇒ nuevo sondeo. `416` con
`next_offset == total` ⇒ segmento completado. Criterio: servidor local que cambia `ETag` y devuelve
`200` a un `If-Range` obsoleto; test que verifica reinicio desde cero y ausencia de mezcla de
generaciones.

### S8 — Reintentos, `Retry-After` y clasificación
Recuperables: reset/pipe roto, timeouts, `429`, `500/502/503/504`. Terminales: `404/410`
(`ERR_RESOURCE_GONE`), `401/403` ⇒ `paused` con `ERR_AUTH_REQUIRED` sin bucle, `ENOSPC`, `EACCES`.
Backoff `min(60 s, 1 s·2^n) ± jitter(0, 0.5·espera)`; `MAX_RETRIES = 5` y luego `paused` con aviso.
`Retry-After` se acepta en segundos o fecha HTTP; fecha inválida o pasada ⇒ 1 s; sin cabecera ⇒ backoff.
`429` reduce conexiones activas de ese dominio. `407` ⇒ `ERR_PROXY_AUTH` en pausa.
Criterio: servidor local con `429 + Retry-After`, `503`, `404` y corte abrupto.

### S9 — Checksum opcional
`expected_checksum {algorithm:"sha256",hash:"..."}` o `Digest: sha-256=`: al terminar todos los
segmentos el estado pasa a `verifying`, se calcula SHA-256 en bloques de 1 MB y coincide ⇒ `completed`;
no coincide ⇒ `failed` con `ERR_CHECKSUM_MISMATCH` y el archivo se renombra con sufijo `.corrupt`.

### S10 — Límite de velocidad global
Token bucket de `R` bytes/s compartido por todos los workers; `0`/`null` = ilimitado; cambio en caliente
sin reiniciar descargas. Criterio: test con bucket de tasa conocida que verifica que la tasa medida no
supera el límite con margen.

### S11 — Seguridad de URL y nombres
Nombres: prioridad `filename*` (RFC 5987) sobre `filename` (RFC 6266); sólo nombre base, sin `/`, `\`,
`../`, `..\\`; controles ASCII (`0x00–0x1F`, `0x7F`) eliminados; `/` y `\0` a `_`; ≤ 255 bytes UTF-8 sin
partir caracteres; nombre vacío o de sólo puntos ⇒ `download_{task_id}.bin`.
SSRF: bloquear loopback `127.0.0.0/8`, `::1`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
`fc00::/7`, `169.254.0.0/16`, `fe80::/10` salvo `allow_local_downloads = true`;
`ERR_BLOCKED_PRIVATE_IP`. Redirecciones: máximo 10 saltos, revalidación SSRF y resolución DNS en cada
salto, sin `Authorization`/`Cookie` en cross-origin, `Referer` actualizado. Sólo `http`/`https`.
TLS nunca con validación desactivada. Criterio: tabla de nombres maliciosos y de IPs bloqueadas.

### S12 — Colisiones de archivo
Si `final_path` existe, se busca `nombre (1).ext`, `nombre (2).ext`, … sin sobrescribir el archivo
existente; la resolución es atómica respecto de otras tareas del daemon. Criterio: test con archivo
preexistente y dos tareas con el mismo nombre.

### S13 — Persistencia SQLite
`$XDG_DATA_HOME/segmenta/segmenta.db`, `PRAGMA journal_mode = WAL`, DDL normativo de 08 (`downloads`,
`segments`, `download_headers`, `download_secrets`, `settings`, `queues`), migraciones idempotentes
verificadas con `user_version`, `next_offset` por segmento, checkpoints con `fdatasync` previo.
Secretos fuera de SQLite: `download_secrets` guarda sólo alias/clave de llavero; si el llavero no está
disponible se usa un archivo `0600` separado (documentado) y nunca una columna con el secreto.
Criterio: tests de migración (abrir dos veces), WAL activo, recuperación de crash (reabrir con estado
`downloading` ⇒ `paused`), y aserción de que ningún valor de secreto aparece en el `.db`.

### S14 — Socket e IPC
Socket `$XDG_RUNTIME_DIR/segmenta/daemon.sock` (fallback `/tmp/segmenta-$UID/`) con directorio `0700` y
socket `0600`; `SO_PEERCRED` y UID distinto ⇒ cierre inmediato (`1010`). NDJSON JSON-RPC 2.0; mensaje
máximo 1 MiB ⇒ `1009`; métodos de 07 con sus códigos `-32700..-32603` y `1001..1010`. Criterio: tests
de framing (línea partida, dos mensajes en un chunk, 1 MiB+, línea vacía), de método desconocido y de
peer UID simulado en la capa de validación.

### S15 — Catálogo de métodos
`download.add|list|status|pause|resume|cancel|delete`, `settings.get|update` (sólo las claves de la
tabla de 07, desconocidas ⇒ `-32602`), `queue.list|create|remove|schedule`,
`system.hello|status|shutdown`. `system.hello` informa `protocol_version: 1`; versión incompatible ⇒
`ERR_PROTOCOL_VERSION`. Criterio: test por método con parámetros válidos e inválidos.

### S16 — Eventos y `seq_id`
Notificaciones `download.event` con `seq_id` monótono, `timestamp_ms`, `event_type`, `task_id` y `data`
según la tabla de payloads de 07. `task_progress` limitado a 4 Hz por descarga. Si el cliente se atrasa
de la cola de 1024 eventos, se emite `system.resync_required` y el cliente recarga con `download.list`.
Criterio: test que verifica monotonía, campos por `event_type` y emisión de `resync_required` al
desbordar.

### S17 — systemd y arranque seguro
Unidades de usuario `segmenta.socket` (`ListenStream=%t/segmenta/daemon.sock`, `SocketMode=0600`,
`DirectoryMode=0700`) y `segmenta.service` (`ExecStart=/usr/bin/segmenta-daemon --systemd`,
`Restart=on-failure`). Fuera de systemd, `flock(LOCK_EX|LOCK_NB)` sobre
`$XDG_RUNTIME_DIR/segmenta/daemon.lock`; segundo proceso ⇒ `ERR_DAEMON_ALREADY_RUNNING`. Criterio: test
de segunda instancia y arranque real por `systemctl --user` en la instalación de prueba.

### S18 — CLI usable
`segmenta add <url>`, `list`, `status <id>`, `pause <id>`, `resume <id>`, `cancel <id>`,
`settings` (`get`/`set`), `--json` en los comandos de lectura. Códigos de salida: `0` éxito, `1` error
de daemon/protocolo, `2` uso inválido. Mensajes legibles en stderr, datos en stdout. No requiere GUI.
Criterio: test de punta a punta contra servidor local y daemon efímero para add→list→pause→resume→cancel.

### S19 — GUI GTK4/Libadwaita
Ventana con `AdwNavigationSplitView` (sidebar de descargas + detalle), estados por ítem, búsqueda y
filtros, acciones pausar/reanudar/cancelar/abrir carpeta/eliminar. Diálogo de alta con selector de
carpeta nativo (`GtkFileDialog`), nombre, conexiones y "comenzar". Progreso, velocidad, ETA y
`SegmentaSegmentView` con un bloque por segmento que se parte en caliente. Preferencias: carpeta,
conexiones, límite de velocidad, categorías y autostart. Integración: `.desktop`, icono, AppStream,
notificaciones, tema claro/oscuro, atajos (`Ctrl+N`, `Ctrl+F`, `Ctrl+,`, `Ctrl+Q`) y estados
vacío/error. Sin Electron ni frameworks pesados. Criterio: smoke headless con `xvfb-run` que construye
la ventana y aplica al menos un evento de progreso; si GTK no arranca se documenta la limitación con el
error real.

### S20 — Empaquetado `.deb`
`cargo-deb` produce un paquete `segmenta` con `/usr/bin/segmenta-daemon`, `/usr/bin/segmenta-cli`,
`/usr/bin/segmenta-gui`, unidades systemd de usuario, `.desktop`, icono, metainfo AppStream y
`/usr/share/doc/segmenta/copyright` con GPL-3.0-or-later. Dependencias de ejecución declaradas sólo si
son reales (`libgtk-4-1`, `libadwaita-1-0`, `libsecret-1-0`, `ffmpeg` como recomendación).
Criterio: `dpkg-deb --info/--contents`, `lintian` si está disponible, e instalación real con
`dpkg -i` + `systemctl --user` en este sistema; se documenta el resultado exacto.

### S21 — Puertas de calidad
`cargo test --workspace`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings` pasan. Sin red externa en tests.
Criterio: los tres comandos en verde en el commit final.

### S22 — Release `v0.0.1`
Tag anotado `v0.0.1` sobre `main` verde y GitHub Release con el `.deb`, su `.sha256` y notas de
instalación. Criterio: `gh release view v0.0.1` lista ambos assets y el checksum verifica.

### S23 — README de release
Instalación en Ubuntu 26.04 (dependencias, `.deb`, `systemctl --user enable --now segmenta.socket`),
uso de CLI y GUI, limitaciones conocidas y qué falta para 0.1.0 (HLS/DASH, extensión, Flatpak).
Criterio: los comandos del README se ejecutan tal cual en esta máquina.

### S24 — Higiene de repositorio
Sin secretos, sin `target/`, `.env`, claves ni `.atl/` versionados; árbol limpio al terminar.
Criterio: `git status --porcelain` vacío y `git ls-files` sin artefactos de build.

### S25 — Honestidad de estado
Toda limitación real del entorno se documenta con el error observado; nada se declara verificado sin
evidencia (capturas, logs, salida de comandos).

## Tasks

Orden TDD: cada tarea arranca con un test que falla (RED) y termina con el mínimo código que lo pone en
verde (GREEN), más refactor con las puertas de S21.

| ID | Specs | Tarea | Rutas | Test RED |
| --- | --- | --- | --- | --- |
| T1 | S1, S2 | Andamiaje del workspace y crates | `Cargo.toml`, `crates/*/Cargo.toml` | `cargo metadata` falla antes de existir |
| T2 | S3, S11 | Servidor de rangos local de pruebas + sondeo y clasificación | `crates/segmenta-core/src/testserver.rs`, `probe.rs`, `url.rs`, `filename.rs` | 206/200/chunked + nombres maliciosos |
| T3 | S4, S5 | Planificador segmentado y hot-split | `planner.rs`, `split.rs` | invariantes sin huecos/solapes y midpoint |
| T4 | S6, S7, S9 | E/S, preasignación, reanudación y checksum | `disk.rs`, `engine.rs`, `checksum.rs` | `pwrite_all` con EINTR/parcial, `If-Range` |
| T5 | S8, S10 | Reintentos, backoff, `Retry-After`, token bucket | `retry.rs`, `ratelimit.rs` | 429/503/404 y tasa medida |
| T6 | S13 | Store SQLite, migraciones, WAL, secretos | `crates/segmenta-daemon/src/store/*` | migración idempotente y secreto ausente en `.db` |
| T7 | S14, S15, S16 | IPC: framing, peercred, métodos, eventos | `crates/segmenta-daemon/src/ipc/*` | 1 MiB, método desconocido, `seq_id` |
| T8 | S12, S17, S7 | Task manager: alta, pausa, reanudación, crash, colisiones | `crates/segmenta-daemon/src/tasks/*` | reabrir tras "crash" ⇒ `paused` y resume |
| T9 | S18 | CLI contra daemon efímero | `crates/segmenta-cli/src/*` | e2e add→list→pause→resume→cancel |
| T10 | S19 | GUI GTK4 + Libadwaita | `crates/segmenta-gui/src/*`, `data/*` | smoke headless bajo `xvfb-run` |
| T11 | S17, S20 | Empaquetado, systemd, `.desktop`, icono, metainfo | `packaging/*`, `crates/segmenta-gui/Cargo.toml` | `dpkg-deb --contents` + instalación |
| T12 | S21, S22, S23 | Puertas verdes, README de release, tag y GitHub Release | `README.md`, `dist/*` | los tres comandos de S21 en verde |

## Presupuesto, riesgos y rollback

- **Presupuesto**: 6–8 h de agente en 12 tareas; corte de alcance en T10/T11 si el núcleo no está verde
  a la mitad del presupuesto. El stretch (`segmenta-host`, `extension/`) no consume presupuesto.
- **Riesgos**: (1) GTK 4.22/libadwaita 1.9 son más nuevos que los crates `gtk4`/`libadwaita`;
  mitigación: usar sólo APIs estables de las features declaradas y bajar de versión de feature si el
  build falla. (2) `cargo-deb` con assets fuera del directorio del crate; mitigación: `--no-build` con
  assets relativos al workspace, o crate de empaquetado dedicado. (3) Sin `Secret Service` en un
  entorno headless; mitigación: fallback documentado a archivo `0600`. (4) Test de `ENOSPC` depende de
  permisos de montaje; mitigación: simulación de error en la capa de syscall.
- **Rollback**: cada tarea es un commit atómico sobre `main`; revertir una tarea es `git revert <sha>`.
  El `.deb` y el tag se crean sólo al final; si una puerta falla no se etiqueta y el release se cancela
  sin dejar artefactos publicados.

## Matriz de pruebas

| Capa | Herramienta | Casos |
| --- | --- | --- |
| Unitaria motor | `cargo test -p segmenta-core` | planificador, split, `pwrite_all`, sanitización, SSRF, backoff, rename de colisión |
| Integración con servidor de rangos | `cargo test -p segmenta-core --test range_download` | 200, 206, 416, 429 + `Retry-After`, ETag cambiado, chunked, corte abrupto |
| SQLite | `cargo test -p segmenta-daemon` | migración, WAL, crash recovery, `next_offset`, secreto fuera de `.db` |
| IPC | `cargo test -p segmenta-daemon --test ipc` | framing, 1 MiB, UID distinto, métodos/errores, eventos/`seq_id` |
| CLI | `cargo test -p segmenta-cli --test cli_e2e` | add/list/status/pause/resume/cancel, `--json`, códigos de salida |
| GUI headless | `xvfb-run -a cargo test -p segmenta-gui` | construcción de ventana, modelo de fila, `SegmentaSegmentView` |
| Empaquetado | `dpkg-deb`, `lintian`, `dpkg -i`, `systemctl --user` | contenido, instalación, arranque del socket |

## Comandos exactos

```bash
# build y puertas
cargo build --release --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# .deb
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
dpkg-deb --info dist/segmenta_0.0.1-1_amd64.deb
dpkg-deb --contents dist/segmenta_0.0.1-1_amd64.deb
lintian dist/segmenta_0.0.1-1_amd64.deb      # si está disponible
sha256sum dist/segmenta_0.0.1-1_amd64.deb > dist/segmenta_0.0.1-1_amd64.deb.sha256

# GUI headless
xvfb-run -a ./target/release/segmenta-gui --smoke

# release
git tag -a v0.0.1 -m "Segmenta 0.0.1"
gh release create v0.0.1 dist/segmenta_0.0.1-1_amd64.deb dist/segmenta_0.0.1-1_amd64.deb.sha256 \
  --title "Segmenta 0.0.1" --notes-file dist/RELEASE_NOTES.md
```

## Criterio de release

`v0.0.1` se etiqueta sólo si: los tres comandos de S21 pasan; un archivo de ≥ 10 MB se descarga
segmentado, se pausa, se reanuda tras matar el daemon y su SHA-256 coincide con el original; la CLI
controla el ciclo completo; la GUI muestra progreso y segmentos bajo `xvfb-run`; el `.deb` se instala y
el daemon arranca por systemd user; el repositorio queda limpio (S24). El tag empuja un GitHub Release
con `.deb`, `.sha256` y notas de instalación (S22).

**L6 — Evidencia de las puertas.** `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings` y `cargo test --workspace` en verde (151 tests, incluidos los 5 de
`cli_e2e`); `xvfb-run -a cargo test -p segmenta-gui` con las 5 pruebas de ventana reales; smoke
`SMOKE OK rows=1 segments=4` 5/5.

**L7 — Defectos encontrados al probar el `.deb` instalado.** (a) `segments.id` es el rowid global:
dos descargas que numeraban sus segmentos desde 0 colisionaban y todo checkpoint de la segunda
fallaba en silencio; ahora se inserta sin id explícito y hay test de regresión con dos tareas.
(b) `segmenta list` imprime ids de 8 caracteres pero los comandos exigían el UUID completo; ahora
se resuelve un prefijo único. (c) un `Completed` no persistía la tabla final de segmentos; (d)
`download_dir` no seguía a `settings`; (e) reanudar sin checkpoint chocaba con `EEXIST`.

**L8 — Verificación sobre el paquete instalado.** `.deb` `segmenta_0.0.1-1_amd64.deb` instalado con
`dpkg -i`, `segmenta.socket` habilitado, `segmenta hello` responde, GUI abre bajo `xvfb`, descarga de
10 MiB con 8 conexiones con SHA-256 idéntico, 32 MiB pausado a ~18 MiB + `SIGKILL` + reconexión +
reanudación con SHA-256 idéntico, y dos descargas simultáneas sin errores de checkpoint. `lintian`
no está instalado en el entorno: no se ejecutó (no se inventa el resultado).

## Log

**L1 (solicitud original del usuario, verbatim):**

> Tomá el frente de Segmenta en `/home/wilkin/proyectos/Segmenta`, sobre `main`, sin ramas ni worktrees. La documentación de v1 ya está cerrada en `docs/openspec/01`–`11`, `README.md` y `LICENSE`. El usuario quiere comenzar la implementación real y dejar una **primera release 0.0.1 lista para probar en su Ubuntu 26.04**.
>
> ## Contexto y corrección de target
>
> La documentación fue escrita para Ubuntu **26.10**; el usuario corrigió el objetivo a **Ubuntu 26.04**. Antes de implementar:
>
> - Actualizá todas las referencias `26.10` → `26.04` en `README.md`, `docs/openspec/01`–`11` y cualquier ejemplo.
> - El entorno actual es Ubuntu **26.04.1 LTS**, sin `cargo` ni GTK instalados, pero con `sudo` disponible y sin display. El build tiene que ser headless; la GUI se prueba con `xvfb-run` si hace falta.
> - Instalá el toolchain y dependencias necesarias antes de empezar:
>   ```bash
>   sudo apt-get update
>   sudo apt-get install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
>     libsecret-1-dev libssl-dev ffmpeg xvfb
>   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
>   source "$HOME/.cargo/env"
>   cargo install cargo-deb
>   ```
>   Si algún paquete no existe en 26.04, reportá el nombre real y buscá el equivalente; no inventes dependencias.
>
> ## Primer artefacto: ODD, antes de código
>
> Creá `odd/tasks/segmenta-v0.0.1.md` y commiteá **solo ese documento** antes de escribir código. El ODD debe incluir:
>
> - objetivo de 0.0.1 y lo que explícitamente NO entra;
> - specs numeradas (S1…Sn) y criterios de aceptación;
> - tareas TDD en orden RED → GREEN → refactor, con archivos y tests;
> - riesgos, rollback, dependencias y presupuesto de tiempo;
> - matriz de pruebas (unitarias, integración con servidor de rangos, CLI, GUI headless, empaquetado);
> - comandos exactos de build, test y `.deb`;
> - criterio de release: tag `v0.0.1` + GitHub Release con `.deb` y SHA-256.
>
> ## Alcance de 0.0.1 (cerrado)
>
> La 0.0.1 debe ser un **gestor de descargas directo usable y nativo**, no una demo:
>
> ### Incluido y obligatorio
>
> 1. **Workspace Rust** con:
>    - `crates/segmenta-core`: motor puro.
>    - `crates/segmenta-daemon`: daemon + IPC + SQLite.
>    - `crates/segmenta-cli`: CLI de control.
>    - `crates/segmenta-gui`: GTK4 + Libadwaita.
>    - `crates/segmenta-host` y `extension/`: **stretch**; sólo si el núcleo, la GUI y el `.deb` ya están verdes. No bloquean la release.
> 2. **Motor HTTP/HTTPS**:
>    - sondeo con `Range`, `Accept-Encoding: identity`, `Content-Range`, `ETag`/`Last-Modified`, `If-Range`;
>    - descarga segmentada con `N` configurable (1–32), fallback single-stream y `chunked`;
>    - hot-split con el protocolo `SplitRequest`/`SplitAck` de 08, sin solapamientos ni huecos;
>    - `pwrite_all` con `EINTR`/partial writes, preasignación `posix_fallocate` con fallback;
>    - reanudación tras pausa/crash, validación de tamaño y checksum opcional;
>    - reintentos/backoff, `Retry-After`, clasificación de errores, límite de velocidad global;
>    - path traversal, SSRF, names sanitizados y colisiones de archivo según 06/08.
> 3. **Daemon, persistencia e IPC**:
>    - SQLite WAL con el DDL de 08, migraciones idempotentes, `next_offset`, `download_headers`/`download_secrets` con keyring cuando haga falta;
>    - socket `$XDG_RUNTIME_DIR/segmenta/daemon.sock` `0600`, `SO_PEERCRED`, NDJSON JSON-RPC 2.0;
>    - métodos de 07: `download.add/list/status/pause/resume/cancel/delete`, `settings.get/update`, `queue.*`, `system.hello/status/shutdown`;
>    - eventos de progreso/estado/segmentos con `seq_id` y resync;
>    - systemd user socket/service de 07 y arranque seguro.
> 4. **CLI usable**:
>    - `segmenta add <url>`, `list`, `status`, `pause`, `resume`, `cancel`, `settings`, `--json` si es razonable;
>    - códigos de salida y mensajes claros; no requiere GUI.
> 5. **GUI GTK4/Libadwaita**:
>    - ventana principal con `AdwNavigationSplitView`, lista de descargas, estados, búsqueda/filtros y acciones;
>    - añadir URL con selector de carpeta nativo, nombre, conexiones y comenzar;
>    - pausar/reanudar/cancelar, progreso, velocidad, ETA y `SegmentaSegmentView` con segmentos;
>    - preferencias básicas: carpeta, conexiones, límite de velocidad, categorías, autostart;
>    - integración Ubuntu: `.desktop`, icono, AppStream/metainfo, notificaciones, portal de archivos, tema claro/oscuro, teclado/accesibilidad y estados vacío/error;
>    - no usar Electron ni dependencias pesadas.
> 6. **Empaquetado y release**:
>    - `cargo build --release --workspace`, `cargo test`, `cargo fmt --check`, `cargo clippy -D warnings`;
>    - `.deb` con `segmenta-core`/daemon/CLI/GUI, systemd user units, `.desktop`, icono, metainfo y `LICENSE`;
>    - instalar y probar el `.deb` localmente o en un contenedor limpio; `dpkg-deb`/`lintian` si está disponible;
>    - tag `v0.0.1` y GitHub Release con el `.deb` + `.sha256` + notas de instalación;
>    - README con instalación, uso, CLI, GUI y limitaciones.
>
> ### Fuera de 0.0.1
>
> - Extensión/Native Messaging y captura de navegador: stretch, no bloquea.
> - HLS/DASH, DRM, streaming, subtítulos y multi-audio: 0.1.0.
> - Flatpak/Snap/AppImage; `.deb` es el formato de esta release.
> - Windows/macOS, telemetría, actualizaciones automáticas y tiendas de extensiones.
>
> ## TDD y calidad
>
> - Trabajá en ciclos pequeños: test que falla → implementación mínima → verde → refactor.
> - Tests de motor con servidor HTTP de rangos local (sin red externa): 200, 206, 416, 429, `Retry-After`, ETag cambiado, `chunked`, corte abrupto, `EINTR`, `ENOSPC`, cero bytes, header malformado.
> - Tests de invariantes de segmentos: sin solapamiento, sin huecos, split en midpoint, clamp de buffer, reanudación.
> - Tests de SQLite: migración, WAL, crash recovery, `next_offset`, secretos fuera de SQLite.
> - Tests de IPC: framing, 1 MiB, UID distinto, métodos/errores, eventos/`seq_id`.
> - Test de CLI de punta a punta contra un servidor local.
> - Smoke de GUI headless con `xvfb-run` (o justificación técnica si GTK no arranca); no inventes capturas.
> - Sin secretos en el repo; no commitees `target/`, `.env`, claves ni `.atl/`.
> - Commits atómicos, mensajes Conventional Commits, todo a `main`. No tag hasta que CI/build/tests estén verdes.
>
> ## Orquestación
>
> - Podés usar subagentes para superficies disjuntas (core, daemon/DB, GUI, packaging) sólo si no se pisan.
> - No mandes a un subagente a implementar sin tests primero.
> - Revisá y commiteá lo de cada worker; no dejes cambios sin commitear.
> - Si algo se bloquea por una dependencia, un fallo de GTK o una decisión de producto, reportá el bloqueo exacto y seguí con la parte independiente.
> - Mantené Herdr abierto; el supervisor revisa cada 5 minutos.
>
> ## Criterio de terminado
>
> 0.0.1 está lista cuando:
>
> 1. `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` pasan.
> 2. Un archivo de prueba de al menos 10 MB se descarga segmentado, se pausa, se reanuda tras matar el daemon y coincide con el SHA-256 original.
> 3. La CLI lista/pausa/reanuda/cancela y la GUI muestra progreso/segmentos sin bloquearse.
> 4. El `.deb` se instala en un entorno limpio, arranca el daemon por systemd user y abre la GUI.
> 5. El repo queda limpio, con tag `v0.0.1` y GitHub Release con el `.deb` y su checksum.
> 6. El README explica cómo instalarlo en Ubuntu 26.04 y qué falta para 0.1.0.
>
> Si no se puede cumplir algo por una limitación real del entorno, no lo maquilles: documentá la limitación, entregá lo que sí funciona y decí exactamente qué falta para probarlo en la máquina del usuario.

**L2 — Exploración inicial.** Repositorio sólo con documentación (3 commits, `main` limpio, remote
`github.com/wilkin-barban/Segmenta`, `gh` autenticado como `wilkin-barban`). Entorno confirmado:
Ubuntu 26.04.1 LTS (`resolute`), x86_64, sin display, GTK 4.22.4, libadwaita 1.9.1, libsecret 0.21.7,
FFmpeg 8.0.1, Rust 1.99.1 instalado por rustup, `xvfb` disponible.

**L3 — Target corregido.** `26.10 → 26.04` aplicado en README y 01/04/05/10/11 (13 líneas), más
`GNOME 48+ → GNOME 50+` en 04 (26.04 LTS embarca GNOME 50.1) y la nota de CI reescrita: 26.04 es la LTS
objetiva, ya estable. Commit `docs: retarget release target to Ubuntu 26.04 LTS`.

**L5 — Códigos de error y evento de reinicio.** El motor necesita identificadores numéricos que 07
no cubre (redirecciones, recurso ausente, cuerpo truncado) y 06 pide notificar
`download_restarted_due_to_stale_cache`, que 07 no lista. Se agregó la tabla 06 §4.3 y el evento 9 en
07 §5 antes de implementarlos, como exige 11 §86.

**L4 — Alcance de 0.0.1 decidido.** Cuatro crates obligatorias; el motor expone su servidor de rangos
de prueba como módulo de `segmenta-core` (feature `testserver`) para que daemon y CLI lo reutilicen sin
una crate extra. `segmenta-host`/`extension` quedan fuera salvo que T1–T11 estén verdes.
