# Segmenta

> Gestor de descargas HTTP/HTTPS segmentado, con partición en caliente y reanudación tras
> caída para **Ubuntu 26.04 LTS (GNOME 50 / Wayland)**, con daemon propio, CLI y GUI
> nativa GTK4 + Libadwaita.

**Estado: 0.0.1** — primera release pública. Descarga directa segmentada, usable de punta a
punta desde la GUI y desde la terminal. Ver [Alcance](#alcance-de-esta-release) para lo que
todavía no entra.

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
- **Checksum opcional** SHA-256 al finalizar, con `ERR_CHECKSUM_MISMATCH` y renombrado a
  `.corrupt` si no coincide.
- **Seguridad**: sin `file:`, `data:` ni `ftp:`; bloqueo de loopback e IPs privadas (SSRF) salvo
  `allow_local_downloads`; sin `Authorization`/`Cookie` al cruzar de origen en una redirección;
  nombres sanitizados (path traversal, 255 bytes, colisiones `archivo (1).ext`); secretos fuera
  de SQLite.
- **CLI y GUI**: control completo desde `segmenta` (sin GUI) y una app GTK4/Libadwaita con
  búsqueda, filtros, estados, progreso, velocidad, ETA y vista de segmentos.

---

## 2. Instalación en Ubuntu 26.04 LTS

### Opción A — paquete `.deb` (recomendado)

Descargá `segmenta_0.0.1_amd64.deb` y su checksum desde la página de releases, verificá y
instalá:

```bash
sha256sum -c segmenta_0.0.1_amd64.deb.sha256
sudo apt install ./segmenta_0.0.1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

El paquete instala `/usr/bin/segmenta-daemon`, `/usr/bin/segmenta-cli`, `/usr/bin/segmenta-gui`
y un enlace `/usr/bin/segmenta`, las unidades de usuario `segmenta.socket`/`segmenta.service`, el
`.desktop`, el icono y el AppStream.

Comprobá que el daemon responde:

```bash
segmenta hello
segmenta system status
```

> El socket es `$XDG_RUNTIME_DIR/segmenta/daemon.sock` con permisos `0600`. `segmenta.socket` es
> quien lo crea; si arrancás sólo `segmenta.service` sin el socket, el daemon falla con
> `address already in use`. Habilita **el socket**.

### Opción B — desde el código

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  libsecret-1-dev libssl-dev xvfb
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

git clone https://github.com/wilkin-barban/Segmenta.git
cd Segmenta
cargo build --release --workspace
sudo install -Dm755 target/release/segmenta-daemon /usr/bin/segmenta-daemon
sudo install -Dm755 target/release/segmenta-cli    /usr/bin/segmenta-cli
sudo install -Dm755 target/release/segmenta-gui    /usr/bin/segmenta-gui
sudo ln -sf segmenta-cli /usr/bin/segmenta
sudo install -Dm644 packaging/segmenta.socket /usr/lib/systemd/user/segmenta.socket
sudo install -Dm644 packaging/segmenta.service /usr/lib/systemd/user/segmenta.service
systemctl --user daemon-reload && systemctl --user enable --now segmenta.socket
```

`ffmpeg` **no** es necesario en 0.0.1: sólo lo usará el muxer HLS/DASH de 0.1.0.

---

## 3. Uso desde la terminal

```bash
# Añadir y seguir una descarga
segmenta add https://example.com/ubuntu.iso
segmenta list
segmenta status <task-id>

# Control
segmenta pause  <task-id>
segmenta resume <task-id>
segmenta cancel <task-id> --delete-file

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

### Límite de velocidad

`global_speed_limit_bps` es global (token bucket compartido por todos los workers) y se puede
cambiar en caliente: `segmenta settings set global_speed_limit_bps 1048576` para 1 MiB/s, `0`
para ilimitado.

---

## 4. Uso de la GUI

`segmenta-gui` o la entrada **Segmenta** del menú de aplicaciones.

- **Lista**: búsqueda (`Ctrl+F`), filtros por estado y colas, progreso, velocidad y ETA.
- **Añadir** (`Ctrl+N`): URL, carpeta destino con el selector nativo del portal, nombre,
  conexiones y "comenzar".
- **Detalle**: barra de progreso, velocidad, ETA, estado y un bloque por segmento que se
  parte en caliente; botones pausar/reanudar/cancelar/eliminar y abrir carpeta.
- **Preferencias** (`Ctrl+,`): carpeta, conexiones, límite de velocidad, categorías y autostart.
- Tema claro/oscuro y color de acento del sistema, navegación completa por teclado, estados
  vacío y error distintos, notificaciones al completar o fallar.

La GUI nunca habla con la red ni con SQLite: es un cliente del socket del daemon, así que podes
cerrarla y las descargas siguen.

---

## 5. Arquitectura

```
crates/
├── segmenta-core      Motor puro: sondeo, planificador, hot-split, pwrite_all, backoff, SSRF
├── segmenta-daemon    SQLite WAL + JSON-RPC sobre Unix socket + orquestación de tareas
├── segmenta-cli       Cliente de terminal (add/list/status/pause/resume/cancel/settings/queues)
└── segmenta-gui       GTK4 + Libadwaita
packaging/             Unidades systemd de usuario, .desktop, AppStream, icono
docs/openspec/         Especificaciones normativas 01-11
odd/tasks/             Documento ODD de la release con specs y evidencia
```

- **Socket**: NDJSON JSON-RPC 2.0 en `$XDG_RUNTIME_DIR/segmenta/daemon.sock` (`0600`), con
  `SO_PEERCRED` y límite de 1 MiB por mensaje.
- **Estado**: `$XDG_DATA_HOME/segmenta/segmenta.db` (WAL, `user_version` para migraciones).
  Los secretos **no** están ahí: se guardan por alias en el Secret Service
  (`secret-tool`) y, si no está disponible, en `$XDG_DATA_HOME/segmenta/secrets.json` con
  permisos `0600`.
- **Eventos**: `download.event` con `seq_id` monótono y `system.resync_required` si un cliente
  se atrasa.

---

## 6. Alcance de esta release

**Incluido**: motor HTTP/HTTPS segmentado con hot-split, reanudación validada, checksum
opcional, límite de velocidad global, reintentos con `Retry-After`, daemon con SQLite WAL e IPC,
CLI completa, GUI GTK4/Libadwaita, `.deb` con unidades systemd de usuario.

**Fuera (0.1.0 en adelante)**:

- **HLS / DASH**, streaming, subtítulos, multi-audio y cualquier contenido con DRM.
- **Extensión de navegador y Native Messaging** (`segmenta-host`, `extension/`): el esqueleto de
  las tablas `download_stream_plan` y `download_type` está, pero no hay captura.
- **Flatpak / Snap / AppImage**, Windows, macOS, telemetría y actualizaciones automáticas.
- **Planificador de colas**: `queue.schedule` persiste la configuración, pero 0.0.1 no arranca
  ni detiene tareas por horario, y `max_active` no limita la concurrencia todavía.
- **Descompresión al vuelo**: si un servidor ignora `Accept-Encoding: identity` y comprime, la
  descarga falla con `ERR_UNSUPPORTED` en lugar de escribir bytes comprimidos.
- **Traducciones gettext**: la UI está en español, sin catálogos `es`/`pt-BR`.

---

## 7. Desarrollo

```bash
cargo build --release --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Pruebas de red con servidor de rangos local (sin red externa), SQLite en directorios temporales,
IPC sobre sockets temporales, GUI con `xvfb-run` y CLI de punta a punta contra un daemon real.

```bash
# Sólo la suite de GUI (necesita display virtual)
xvfb-run -a cargo test -p segmenta-gui

# Paquete
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
dpkg-deb --info dist/segmenta_0.0.1_amd64.deb
sha256sum dist/segmenta_0.0.1_amd64.deb > dist/segmenta_0.0.1_amd64.deb.sha256
```

---

## 8. Documentación

1. [`docs/openspec/11-decision-log.md`](docs/openspec/11-decision-log.md) — alcance y precedencia.
2. [`docs/openspec/05-implementation-roadmap-for-agents.md`](docs/openspec/05-implementation-roadmap-for-agents.md) — orden de fases.
3. `06` HTTP, `07` IPC, `08` persistencia y hot-split, `10` empaquetado.
4. [`odd/tasks/segmenta-v0.0.1.md`](odd/tasks/segmenta-v0.0.1.md) — plan y evidencia de esta release.

Licencia: GPL-3.0-or-later (ver [`LICENSE`](LICENSE)).
