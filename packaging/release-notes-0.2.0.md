# Segmenta 0.2.0

**En preparación.** Este documento describe lo aprobado para 0.2.0, no lo publicado: todavía no
hay tag `v0.2.0`, ni Release, ni `.deb` `dist/segmenta_0.2.0-1_amd64.deb`. Nada de lo que sigue
debe presentarse como comportamiento disponible hasta que Q1 cierre los gates y el QA real en
Ubuntu. La última release publicada es 0.1.2.

0.2.0 cierra los gaps de confiabilidad del daemon y agrega las funciones aprobadas en el plan
`segmenta-0.2`: scheduler de colas nativo, `max_active` real, post-procesamiento seguro con
ClamAV, site logins Basic y hardening de la extensión.

## Qué cambia

### Scheduler de colas y `max_active`

- Loop nativo `tokio` dentro del daemon que lee las ventanas horarias persistidas de cada cola y
  arranca o detiene tareas según la hora local.
- `max_active` pasa a ser un límite real por cola: el daemon no arranca más tareas de las
  configuradas, y al completar, fallar o cancelar una tarea libera el slot y despierta al
  scheduler.
- `queue.schedule` y `max_active` siguen siendo la fuente de verdad; la GUI no duplica ese
  estado.
- `action_on_complete` (`none`/`suspend`/`power_off`/`exit`) se ejecuta una sola vez, sólo cuando
  la cola queda vacía. Suspender o apagar requiere permiso del sistema y no se prueba de forma
  destructiva en CI.

### Post-procesamiento y ClamAV

- El hook de post-procesamiento es argv, nunca shell: sólo se sustituyen `%file%`, `%folder%` y
  `%name%`, con timeout y salida acotada. Funciona con rutas que tienen espacios.
- ClamAV integrado y opcional: se detecta `clamdscan` y después `clamscan`; settings
  `antivirus_enabled` (por defecto `false`), `antivirus_timeout_secs` y
  `antivirus_quarantine_dir`.
- Flujo `finalizing` → `verifying` → escaneo/hook → `completed` o cuarentena. Un archivo
  infectado se mueve a cuarentena y la tarea queda `failed` con `ERR_ANTIVIRUS_FOUND` (1046); el
  archivo nunca se deja en Descargas ni se publica como completado.
- Si el antivirus está activado y no hay motor, la tarea falla con `ERR_ANTIVIRUS_UNAVAILABLE` en
  lugar de completarse sin escanear. Un fallo del hook o del escáner no borra el archivo original.
- Migración de esquema a v4 con `quarantine_path`, `postprocess_exit` y `postprocess_message`;
  la GUI muestra el estado «En cuarentena» y su ruta.

### Site logins Basic con GNOME Keyring

- RPC y UI para guardar credenciales por dominio exacto (sin comodines): el usuario queda en
  SQLite y la contraseña en el keyring con alias `site/<dominio>`.
- Al añadir, iniciar o probar una URL con login del host se inyecta `Authorization: Basic` como
  cabecera sensible; una cabecera explícita de la tarea gana. Se mantiene la regla de no reenviar
  credenciales en redirecciones cross-origin.
- Sin keyring disponible, la operación falla con `ERR_KEYRING_UNAVAILABLE` y no crea la entrada.
  La contraseña nunca se devuelve por RPC, no aparece en `settings.get` ni en los logs.
- Si el servidor responde 401 sin login configurado se mantiene `ERR_AUTH_REQUIRED`; no hay
  fallback silencioso a Basic.
- **Sin Digest**: sólo Basic. Es una limitación documentada, no una promesa.

### Extensión, captura e instalación manual

- La instalación sigue siendo manual y asistida por la GUI: `extension.status`,
  `extension.prepare` y `extension.open` preparan y abren la página de extensiones; no hay Chrome
  Web Store ni AMO.
- El sniffer se mantiene **sólo en YouTube**; la captura genérica de descargas existente es la
  vía para el resto de sitios. No se agregan adaptadores para Vimeo ni Twitter.
- Reconciliación al arrancar en frío: al iniciar el worker (`runtime.onStartup`/`onInstalled` y un
  tick del watchdog) se buscan descargas en curso, se reutiliza la decisión de captura y la
  denylist, y se rehidratan las capturas pendientes sin duplicar tareas ni pausar de más.
- La captura mantiene la confirmación con watchdog: aceptar crea la tarea antes de cancelar y
  borrar la copia del navegador; rechazar, timeout, ventana cerrada o host caído reanudan la
  descarga nativa.

### Hardening

- Probe multiarchivo de `.torrent`: se leen los archivos del `info` y se devuelven `total_bytes`
  como suma, `files[]` con índice, ruta, longitud y selección, y `name`; el caso de un solo
  archivo se conserva.
- Ayuda CLI completa en es/en/pt-BR, con `extension` y sus flags visibles en `--help`, y un test
  que falla si un subcomando registrado no aparece.
- Sin categorías automáticas: se retiran de la UI y de las specs, se conserva la lectura legacy
  para no romper configuraciones existentes.

### Documentación

- README, CHANGELOG, specs `01`–`12` y `docs/qa/` alineados con el comportamiento real y con esta
  release. Ningún cambio de código se considera terminado si su contrato en `docs/openspec` no
  está actualizado en el mismo commit.

## Requisitos

- Ubuntu 26.04 (paquete `.deb`).
- `yt-dlp >= 2026.10.01` y `ffmpeg` en `PATH`.
- `libsecret-tools` para el Secret Service del escritorio.
- ClamAV es **opcional y externo**: el `.deb` lo recomienda, no lo embebe. Sin motor instalado y
  con `antivirus_enabled = true`, las tareas fallan con `ERR_ANTIVIRUS_UNAVAILABLE`.
- Chrome/Chromium/Edge/Brave/Vivaldi/Opera o Firefox oficial; la extensión se carga a mano.

## Instalación prevista

Cuando Q1 construya y valide el paquete:

```bash
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0

sudo dpkg -i dist/segmenta_0.2.0-1_amd64.deb
sudo apt-get install -f
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

`dist/segmenta_0.2.0-1_amd64.deb` no existe todavía; los artefactos publicados hoy son los de
0.1.2.

## Verificación antes de etiquetar

La release no se publica si falla cualquier gate o el QA real:

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` sin skips silenciosos (GUI con `xvfb-run`).
- `node extension/tests/run.mjs`, `node extension/tools/sync-shared.mjs --check`,
  `node extension/tools/pack.mjs --check`.
- `packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0` en 22/22.
- QA en Ubuntu: instalación 0.2.0, `extension.status/prepare/open` en Chrome for Testing y
  Firefox oficial, panel 4K/1080p/720p/360p + audio, captura con aceptar/rechazar/timeout/host
  caído, cold start, scheduler con ventana corta y `max_active`, ClamAV con fixture EICAR y
  cuarentena, site login Basic contra servidor local, torrent multiarchivo y `extension` en
  `--help`.
- Limpieza final: procesos, audio, fixtures, tareas de prueba y `allow_local_downloads=false`.

## Límites conocidos

- **Scheduler**: sólo corre mientras el daemon está activo. La activación por socket lo despierta,
  pero una ventana horaria no se ejecuta si no hay proceso.
- **ClamAV**: externo y opcional; la calidad depende de firmas actualizadas. El `.deb` no puede
  embeberlo ni garantizar que esté configurado.
- **Site logins**: Basic sin Digest no cubre todos los servidores. Los sitios que exigen Digest
  siguen fallando con `ERR_AUTH_REQUIRED`.
- **Extensión sin tienda**: Chrome estable no permite instalar sin tienda o política; la GUI sólo
  prepara y abre. Firefox sin firma se carga de forma temporal.
- **Firefox snap** no puede ejecutar el host nativo; usá Firefox oficial o el `.deb`.
- **Cold start**: si el navegador entrega la descarga antes de que exista el service worker, la
  carrera es irreducible; la reconciliación reduce la ventana, no la elimina mágicamente.
- **Medios**: sniffer sólo YouTube; DASH con `SegmentList`/`SegmentBase`, DRM, AES-128, live,
  subtítulos y multi-audio siguen fuera de alcance.
- **Formato de instalación**: `.deb` para Ubuntu 26.04; sin Flatpak, Snap, AppImage ni otros
  sistemas.

## Artefactos

Todavía no hay artefactos 0.2.0. Al cerrar Q1 se publicarán `segmenta_0.2.0-1_amd64.deb` con su
`.sha256`, `segmenta-extension-0.2.0.zip` y `segmenta-firefox-0.2.0.zip`, con los SHA-256 reales
en `docs/qa/segmenta-0.2-live.md` y en el CHANGELOG.
