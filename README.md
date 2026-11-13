# Segmenta

> Gestor de descargas HTTP/HTTPS segmentado, torrents, YouTube y HLS/DASH públicos para
> **Ubuntu 26.04 LTS (GNOME 50 / Wayland)**, con daemon propio, CLI, GUI nativa GTK4 +
> Libadwaita y extensión de navegador.

**Estado: 0.2.0 construida y verificada, pendiente de publicación.** El `.deb` y los dos ZIP están
en `dist/` con sus SHA-256 y pasaron el QA real en Ubuntu (`docs/qa/segmenta-0.2-live.md`); el tag
`v0.2.0` todavía no existe. La última release publicada sigue siendo **0.1.2** (tag `v0.1.2`). El
`.deb` 0.2.0 y los dos ZIP **sí están construidos y verificados** en `dist/` (con sus SHA-256 y el
QA real de `docs/qa/segmenta-0.2-live.md`); lo que falta es el tag y la Release, no el paquete. Lo
que sigue describe el comportamiento real
de 0.1.2 y separa explícitamente lo que entra en 0.2.0 cuando el QA en Ubuntu cierre.

### Novedades de 0.1.2

- **Estado real de la extensión**: la GUI/Navegadores distingue manifiesto nativo listo con
  extensión no instalada, extensión instalada, manifiesto ausente y navegador no soportado
  (snap/Flatpak), por navegador.
- **`segmenta extension status|prepare|install-native|open`**: prepara el ZIP en
  `$XDG_DATA_HOME/segmenta/extensions/<familia>-<versión>`, valida `manifest.json`, `_locales` e
  identificador y abre `chrome://extensions`, `about:debugging` o la carpeta con argv fijo.
- **Panel de calidades acotado**: 4K (2160), 1080p, 720p, 360p y solo audio, sólo las
  disponibles, con 720p por defecto; el vídeo se elige por altura, nunca por `format_id`.
- **Captura con confirmación**: una descarga de `.zip`, `.rar`, `.deb` y compañía se pausa y abre
  una ventana con nombre, host y carpeta de destino; aceptar crea la tarea y sólo entonces se
  cancela y borra la copia del navegador.
- **Watchdog con `alarms`**: cada captura pendiente se persiste antes de pausar y se rehidrata al
  despertar el service worker o al arrancar el navegador; rechazo, timeout (~120 s), ventana
  cerrada o host caído reanudan la descarga del navegador.
- **Orden y foco**: una tarea nueva aparece arriba y `system.open` trae la GUI al frente con la
  tarea enfocada.

### Novedades de 0.1.0 y 0.1.1

0.1.1 sumó la extensión de navegador con host nativo, la captura de descargas y el motor de
torrents (`librqbit`). 0.1.0 sumó YouTube, HLS/DASH públicos y el panel flotante. El detalle
completo está en [`CHANGELOG.md`](CHANGELOG.md).

### Qué entra en 0.2.0

Estas funciones ya están en el código de este repositorio y pasaron el QA real registrado en
`docs/qa/segmenta-0.2-live.md`; lo que sigue pendiente es la publicación (tag y Release):

- **Scheduler de colas nativo**: ventanas horarias por cola y `max_active` real aplicado por el
  daemon.
- **Post-procesamiento argv seguro y ClamAV integrado**: hook sin shell, escaneo opcional con
  `clamdscan`/`clamscan`, cuarentena y `ERR_ANTIVIRUS_FOUND`.
- **Site logins Basic** por dominio con GNOME Keyring (sin Digest).
- **Sin categorías automáticas**, sniffer sólo YouTube, instalación manual de la extensión
  asistida por la GUI y reconciliación de capturas al arrancar en frío.

---

## 1. Qué hace

- **Descarga segmentada**: sondea el recurso con `Range: bytes=0-1` y, si el servidor
  responde `206`, reparte el archivo en hasta 32 conexiones. Si el servidor ignora `Range`,
  cae a un solo stream; si responde `Transfer-Encoding: chunked`, descarga en streaming hasta
  el cierre.
- **Partición en caliente**: cuando un worker termina su rango, en lugar de quedar ocioso pide
  al worker más lento la mitad de sus bytes restantes (`SplitRequest`/`SplitAck`). El reparto
  no deja huecos ni solapamientos.
- **Sin ensamblado**: cada byte se escribe con `pwrite64` en su offset final de un único
  archivo, preasignado con `posix_fallocate` (con fallback a `set_len` + comprobación de
  espacio).
- **Reanudación real**: el estado vive en SQLite (WAL) con `next_offset` por segmento. Pausar,
  cerrar la GUI, matar el daemon o reiniciar la sesión no pierde bytes: al reanudar se manda
  `If-Range` con el validador y, si el archivo remoto cambió, se reinicia desde cero en lugar
  de mezclar generaciones.
- **Torrents**: motor embebido `librqbit` con una sesión compartida por proceso, schema v3
  (`torrents`, `torrent_files`), RPC y CLI `torrent probe|add|status`, saneamiento de rutas
  internas (`..`, absolutas, escapes) y seeding desactivado por defecto.
- **YouTube y HLS/DASH públicos**: `media.probe`/`media.add` con `yt-dlp` para video, audio sin
  recodificar y playlists con límite (1–500); HLS/DASH con `SegmentTemplate` se bajan por
  segmentos y se remuxean con `ffmpeg -c copy`. DRM, AES-128, live, subtítulos y multi-audio se
  rechazan con código explícito.
- **Extensión de navegador y host nativo**: panel en YouTube, captura genérica de descargas,
  botón de magnet en el menú contextual y comunicación con el daemon a través de
  `segmenta-host` (Native Messaging, sin shell).
- **Seguridad**: sin `file:`, `data:` ni `ftp:`; bloqueo de loopback e IPs privadas (SSRF) salvo
  `allow_local_downloads`; sin `Authorization`/`Cookie` al cruzar de origen en una redirección;
  nombres sanitizados (path traversal, 255 bytes, colisiones `archivo (1).ext`); secretos fuera
  de SQLite.
- **CLI y GUI**: control completo desde `segmenta` (sin GUI) y una app GTK4/Libadwaita con
  búsqueda, filtros, estados, progreso, velocidad, ETA y vista de segmentos.

---

## 2. Instalación en Ubuntu 26.04 LTS

### Opción A — el `.deb` que construye este repositorio (recomendado)

El paquete de esta release se construye en este repositorio y se instala con `dpkg`; no hay PPA
ni otro canal de distribución.

```bash
git clone https://github.com/wilkin-barban/Segmenta.git
cd Segmenta

# Construir el .deb de la versión del workspace (0.2.0)
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/

# Puerta de empaquetado antes de instalar
packaging/check-package.sh dist/segmenta_0.1.2-1_amd64.deb 0.1.2

sudo dpkg -i dist/segmenta_0.1.2-1_amd64.deb
sudo apt-get install -f          # resuelve dependencias si faltaba alguna

systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

El flujo apunta a `dist/segmenta_0.2.0-1_amd64.deb` con
`packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0`, que valida 22/22 rutas. El
archivo existe en `dist/` con su SHA-256, pero no hay tag `v0.2.0`, ni Release, ni asset que
descargar todavía.

El paquete instala `/usr/bin/segmenta-daemon`, `/usr/bin/segmenta-cli`, `/usr/bin/segmenta-gui`,
`/usr/bin/segmenta-host`, el enlace `/usr/bin/segmenta`, las unidades de usuario
`segmenta.socket`/`segmenta.service`, los siete manifiestos nativos de mensajería, el `.desktop`,
el handler `magnet:`, el icono, el AppStream y los dos ZIP de la extensión en
`/usr/share/segmenta/extensions/`.

Comprobá que el daemon responde:

```bash
segmenta hello
segmenta system status
```

> El socket es `$XDG_RUNTIME_DIR/segmenta/daemon.sock` con permisos `0600`. `segmenta.socket` es
> quien lo crea; si arrancás sólo `segmenta.service` sin el socket, el daemon falla con
> `address already in use`. Habilita **el socket**.

### Opción B — desde el código, sin empaquetar

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  libsecret-1-dev libssl-dev xvfb ffmpeg yt-dlp libsecret-tools
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

cargo build --release --workspace
sudo install -Dm755 target/release/segmenta-daemon /usr/bin/segmenta-daemon
sudo install -Dm755 target/release/segmenta-cli    /usr/bin/segmenta-cli
sudo install -Dm755 target/release/segmenta-gui    /usr/bin/segmenta-gui
sudo install -Dm755 target/release/segmenta-host   /usr/bin/segmenta-host
sudo ln -sf segmenta-cli /usr/bin/segmenta
sudo install -Dm644 packaging/segmenta.socket  /usr/lib/systemd/user/segmenta.socket
sudo install -Dm644 packaging/segmenta.service /usr/lib/systemd/user/segmenta.service
systemctl --user daemon-reload && systemctl --user enable --now segmenta.socket
```

`ffmpeg` hace falta para HLS/DASH y para el merge de video+audio; `yt-dlp` debe ser
**>= 2026.10.01** (el 2026.03 de Ubuntu devuelve 403 y formatos obsoletos). `libsecret-tools`
es la vía real al Secret Service del escritorio.

---

## 3. Extensión de navegador y host nativo

La extensión **no se instala sola**: el `.deb` deja el host nativo, los manifiestos y los ZIP, y
la carga en el navegador es un paso manual asistido por la GUI. No hay Chrome Web Store ni AMO
en 0.1.2 ni en 0.2.0.

```bash
# Estado por navegador: manifiesto nativo, extensión instalada y conexión
segmenta extension status

# Descomprimir el ZIP en $XDG_DATA_HOME/segmenta/extensions/<familia>-<versión>
segmenta extension prepare --family all      # o chromium / firefox

# Abrir la página de extensiones del navegador (argv fijo, sin shell)
segmenta extension open --browser chrome     # brave, edge, vivaldi, opera, firefox…
```

Después, en el navegador:

- **Familia Chromium** (Chrome, Chromium, Brave, Edge, Vivaldi, Opera): `chrome://extensions` →
  activar **Modo de desarrollador** → **Cargar descomprimida** → la carpeta
  `…/segmenta/extensions/chromium-<versión>` que preparó el comando anterior. Chrome estable
  puede requerir este paso una vez; no se escriben perfiles ni se usa `--load-extension`.
- **Firefox**: `about:debugging#/runtime/this-firefox` → **Cargar complemento temporal** →
  `manifest.json` de la carpeta `…/segmenta/extensions/firefox-<versión>`. El identificador es
  `segmenta@wilkin.github.io`; sin firma, la carga es temporal.

Los ZIP reproducibles son `dist/segmenta-extension-0.1.2.zip` (Chromium) y
`dist/segmenta-firefox-0.1.2.zip` (Firefox); Q1 generará los `0.2.0` con el mismo
`node extension/tools/pack.mjs`. El `.deb` también los instala en
`/usr/share/segmenta/extensions/`.

Si el manifiesto nativo de tu usuario falta, `segmenta extension install-native` lo escribe en
`$XDG_CONFIG_HOME` para los navegadores detectados (o `extension/install-native.sh` desde el
árbol, que además acepta `--print`). El manifiesto del sistema viaja en el `.deb`.

### Descargas del navegador (captura genérica)

- La extensión escucha `downloads.onCreated` y captura por extensión con una lista por defecto
  editable en Preferencias y una denylist. `r0*`/`r1*` se interpretan como prefijos.
- Con `capture_confirm` activo (por defecto), la descarga del navegador se **pausa**, se abre una
  ventana con el nombre, el host y la carpeta de destino, y sólo al aceptar se crea la tarea en
  Segmenta; recién entonces se cancela y borra la copia del navegador.
- **Nunca se pierde un archivo**: rechazar, el timeout (~120 s), cerrar la ventana, un fallo del
  host o una descarga que no se puede pausar reanudan la copia del navegador.
- **Watchdog**: cada captura pendiente se guarda antes de pausar y un `alarm` la vigila; al
  despertar el service worker o al arrancar el navegador se rehidrata. Si el registro o el alarm
  no se pueden crear, no se pausa nada.
- `.torrent` se deriva a `torrent.add`; los enlaces `magnet:` se ofrecen en el menú contextual y
  por el handler `segmenta-handler.desktop` (si tu escritorio no lo registra solo:
  `xdg-mime default segmenta-handler.desktop x-scheme-handler/magnet`).
- Con `capture_confirm = false` se vuelve al flujo sin ventana, que sigue esperando la tarea
  antes de cancelar la descarga nativa.

**Límite conocido**: una descarga que arranca en el mismo instante en que arranca el navegador
puede ocurrir antes de que el service worker registre su listener. Las descargas posteriores sí
se capturan; 0.2.0 agrega una reconciliación con `downloads.search` que reduce la ventana, sin
eliminarla del todo.

---

## 4. Uso desde la terminal

```bash
# Añadir y seguir una descarga
segmenta add https://example.com/ubuntu.iso
segmenta list
segmenta status <task-id>

# Control
segmenta pause  <task-id>
segmenta resume <task-id>
segmenta cancel <task-id> --delete-file
segmenta delete <task-id> --delete-file

# Opciones de alta
segmenta add https://example.com/file.zip \
  --dir ~/Descargas --name file.zip --connections 8 \
  --sha256 <hex> --queue main

# Configuración (los mismos valores que expone la GUI)
segmenta settings
segmenta settings set max_connections 8 global_speed_limit_bps 2000000

# Colas
segmenta queues list
segmenta queues create noche --max-active 1
segmenta queues schedule noche --start 02:00 --stop 06:00 --on-complete suspend

# Medios: YouTube y HLS/DASH públicos
segmenta media probe https://www.youtube.com/watch?v=<id>
segmenta media add   https://www.youtube.com/watch?v=<id> --mode video --height 720
segmenta media add   https://www.youtube.com/watch?v=<id> --mode audio
segmenta media add   https://www.youtube.com/playlist?list=<id> --mode playlist --limit 10
segmenta media add   https://example.com/manifest.m3u8   # HLS/DASH: el daemon detecta el manifiesto

# Torrents
segmenta torrent probe https://example.com/file.torrent
segmenta torrent add   "magnet:?xt=urn:btih:<hash>" --dir ~/Descargas
segmenta torrent status <task-id>

# Extensiones
segmenta extension status
segmenta extension prepare --family chromium
segmenta extension install-native
segmenta extension open --browser chrome

# Daemon
segmenta hello
segmenta system status
segmenta system shutdown
```

Todos los comandos de lectura aceptan `--json`:

```bash
segmenta --json list | jq '.[] | {id: .task_id, status, pct: (.downloaded_bytes / .total_bytes)}'
```

Códigos de salida: `0` éxito, `1` error del daemon o del protocolo, `2` uso inválido.

`extension` aparece en `segmenta --help` desde 0.2.0; `segmenta extension --help` muestra sus
cuatro acciones.

### Límite de velocidad

`global_speed_limit_bps` es global (token bucket compartido por todos los workers) y se puede
cambiar en caliente: `segmenta settings set global_speed_limit_bps 1048576` para 1 MiB/s, `0`
para ilimitado.

---

## 5. Uso de la GUI

`segmenta-gui` o la entrada **Segmenta** del menú de aplicaciones.

- **Lista**: búsqueda (`Ctrl+F`), filtros por estado y colas, progreso, velocidad y ETA; la tarea
  más nueva aparece arriba.
- **Añadir** (`Ctrl+N`): URL, carpeta destino con el selector nativo del portal, nombre,
  conexiones y "comenzar".
- **Detalle**: barra de progreso, velocidad, ETA, estado y un bloque por segmento que se
  parte en caliente; botones pausar/reanudar/cancelar/eliminar y abrir carpeta.
- **Preferencias** (`Ctrl+,`): General (carpeta, conexiones, límite de velocidad, autostart),
  Navegadores (estado real por navegador, preparar extensión, abrir página de extensiones,
  copiar ruta, instalar manifiestos) y Descargas web (activar captura, `.torrent`/magnet, lista
  de extensiones).
- Tema claro/oscuro y color de acento del sistema, navegación completa por teclado, estados
  vacío y error distintos, notificaciones al completar o fallar.

La GUI nunca habla con la red ni con SQLite: es un cliente del socket del daemon, así que podés
cerrarla y las descargas siguen.

---

## 6. Arquitectura

```
crates/
├── segmenta-core      Motor puro: sondeo, planificador, hot-split, pwrite_all, backoff, SSRF
├── segmenta-daemon    SQLite WAL + JSON-RPC sobre Unix socket + orquestación, torrents y medios
├── segmenta-cli       Cliente de terminal (add/list/status/pause/resume/cancel/settings/…)
├── segmenta-gui       GTK4 + Libadwaita
└── segmenta-i18n      Catálogos es/pt-BR/en
extension/             Extensión Chromium MV3 y WebExtension Firefox, tests y empaquetado
packaging/             Unidades systemd de usuario, .desktop, AppStream, icono, notes, gates
docs/openspec/         Especificaciones normativas 01-11
docs/qa/               Evidencia de QA por release
odd/tasks/             Documento ODD de cada release con specs y evidencia
```

- **Socket**: NDJSON JSON-RPC 2.0 en `$XDG_RUNTIME_DIR/segmenta/daemon.sock` (`0600`), con
  `SO_PEERCRED` y límite de 1 MiB por mensaje.
- **Estado**: `$XDG_DATA_HOME/segmenta/segmenta.db` (WAL, `user_version` para migraciones).
  Los secretos **no** están ahí: se guardan por alias en el Secret Service
  (`secret-tool`); sin llavero, un alta con credenciales falla con
  `ERR_KEYRING_UNAVAILABLE` (1031) en lugar de escribir un archivo.
- **Eventos**: `download.event` con `seq_id` monótono y `system.resync_required` si un cliente
  se atrasa.

---

## 7. Alcance y límites

**Incluido en 0.1.2**: motor HTTP/HTTPS segmentado con hot-split, reanudación validada, checksum
opcional, límite de velocidad global, reintentos con `Retry-After`, torrents, YouTube y HLS/DASH
públicos, extensión de navegador con host nativo, captura con confirmación y watchdog, daemon con
SQLite WAL e IPC, CLI completa, GUI GTK4/Libadwaita, `.deb` y dos ZIP de extensión.

**Límites reales de 0.1.2**:

- **Planificador de colas**: `queue.schedule` y `max_active` se persisten, pero el daemon todavía
  no abre ni cierra ventanas ni limita la concurrencia. Está aprobado para 0.2.0.
- **Antivirus y post-procesamiento**: no existen todavía; no hay cuarentena ni `ERR_ANTIVIRUS_FOUND`
  en 0.1.2. Están aprobados para 0.2.0.
- **Site logins**: no hay credenciales por dominio; las credenciales de una tarea se pasan como
  cabeceras explícitas al darla de alta. Basic está aprobado para 0.2.0, sin Digest.
- **DASH `SegmentList`/`SegmentBase`** queda fuera de alcance (`1043`); se soporta
  `SegmentTemplate`. DRM, AES-128, streams en vivo, subtítulos y multi-audio se rechazan.
- **Probe de `.torrent` multiarchivo**: la descarga multiarchivo funciona, pero `torrent probe`
  todavía informa un único archivo de longitud 0 para esos `.torrent`; R2 lo corrige en 0.2.0.
- **Extensión**: sólo YouTube en el sniffer; no hay Vimeo, Twitter ni adaptadores nuevos. Sin
  firmas ni tiendas: Chrome estable exige **Cargar descomprimida** y Firefox una carga temporal.
- **Firefox snap** no puede ejecutar el host nativo; usá el Firefox oficial o el `.deb`.
- **Cold start** de la captura: carrera irreducible si el navegador entrega la descarga antes de
  que exista el service worker (ver §3).
- **Descompresión al vuelo**: si un servidor ignora `Accept-Encoding: identity` y comprime, la
  descarga falla con `ERR_UNSUPPORTED` en lugar de escribir bytes comprimidos.
- **Otros formatos**: Flatpak, Snap, AppImage, Windows, macOS, telemetría y actualizaciones
  automáticas no están soportados.

---

## 8. Desarrollo

```bash
cargo build --release --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# Extensión
node extension/tests/run.mjs
node extension/tools/sync-shared.mjs --check
node extension/tools/pack.mjs --check
```

Pruebas de red con servidor de rangos local (sin red externa), SQLite en directorios temporales,
IPC sobre sockets temporales, GUI con `xvfb-run` y CLI de punta a punta contra un daemon real.

```bash
# Sólo la suite de GUI (necesita display virtual)
xvfb-run -a cargo test -p segmenta-gui

# Paquete
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.1.2-1_amd64.deb 0.1.2
sha256sum dist/segmenta_0.1.2-1_amd64.deb > dist/segmenta_0.1.2-1_amd64.deb.sha256
```

---

## 9. Documentación

1. [`docs/openspec/11-decision-log.md`](docs/openspec/11-decision-log.md) — alcance y precedencia.
2. [`docs/openspec/05-implementation-roadmap-for-agents.md`](docs/openspec/05-implementation-roadmap-for-agents.md) — orden de fases.
3. `01` arquitectura, `02` motor, `03` extensión y Native Messaging, `04` UI/UX, `06` HTTP,
   `07` IPC, `08` persistencia y recuperación, `09` seguridad de la extensión, `10` empaquetado
   y pruebas.
4. [`CHANGELOG.md`](CHANGELOG.md) — cambios por release.
5. [`docs/qa/segmenta-0.1.2-live.md`](docs/qa/segmenta-0.1.2-live.md) — evidencia del QA de 0.1.2.
6. [`odd/tasks/segmenta-0.1.2.md`](odd/tasks/segmenta-0.1.2.md) — plan y evidencia de 0.1.2;
   [`odd/tasks/segmenta-0.2.md`](odd/tasks/segmenta-0.2.md) — plan de 0.2.0.
7. [`packaging/release-notes-0.1.2.md`](packaging/release-notes-0.1.2.md) y
   [`packaging/release-notes-0.2.0.md`](packaging/release-notes-0.2.0.md) — notas de cada release.

Licencia: GPL-3.0-or-later (ver [`LICENSE`](LICENSE)).
