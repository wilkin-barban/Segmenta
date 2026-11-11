# Segmenta 0.0.2

Segunda release: corrige los defectos P0/P1 encontrados al probar la `0.0.1` en Ubuntu 26.04.1 LTS
(GNOME 50 / Wayland) y agrega logging auditable, i18n y tema claro/oscuro verificable. El alcance
no cambia: sólo descargas directas HTTP/HTTPS.

## Qué cambia

### Secretos

- El keyring se detecta de verdad: `secret-tool` presente más una consulta de sondeo que termina
  con código 0 (secreto encontrado) o 1 (no hay secreto). Se eliminó la comprobación con
  `secret-tool --version`, que en libsecret 0.21 devuelve 2 y hacía que el daemon eligiera siempre
  el archivo. `segmenta system status` informa `secret_backend=keyring` cuando el Secret Service
  funciona.
- **Sin fallback silencioso a archivo**: el backend por defecto es el keyring o «no disponible».
  Un `download.add` con `Authorization`, `Cookie` o `cookies[]` sin keyring falla con `1031`
  (`ERR_KEYRING_UNAVAILABLE`) y no crea la tarea; una tarea existente cuyo secreto deja de poder
  leerse pasa a `paused` con `1031`.
- El `.deb` declara `libsecret-tools` en `Depends`.

### Directorio de descargas

- `default_download_dir()` resuelve `XDG_DOWNLOAD_DIR`, `xdg-user-dir DOWNLOAD`,
  `~/.config/user-dirs.dirs` y recién entonces `$HOME/Downloads`. En un Ubuntu en español una
  descarga sin `-d` cae en `~/Descargas`.

### Estados de las tareas

- `pause` sólo se acepta en `downloading`, `pending`, `probing`, `retrying`, `verifying` y
  `finalizing`; `resume` sólo en `paused` y `retrying`. En estados terminales devuelven `1011`
  (`TASK_NOT_PAUSABLE`) y `1003` (`TASK_NOT_RESUMABLE`) sin tocar el estado ni emitir eventos.
- Al pausar, el status de la tarea deja `speed_bps = 0` y `eta_seconds = null`.

### GUI y empaquetado

- La ventana arranca de verdad: con el daemon activo lista una tarea completada en menos de 2 s
  (antes quedaba en «Conectando con el daemon…»). Si el socket no está activo muestra el motivo,
  el comando accionable `systemctl --user enable --now segmenta.socket` y un botón **Reintentar**
  que reconecta y recarga la lista.
- `segmenta-gui --version` y `segmenta-gui --help` (documenta `--smoke` y `--screenshots`).
- Apariencia real: `settings.appearance` = `system`|`light`|`dark` sobre `AdwStyleManager`, y
  `segmenta-gui --screenshots <dir>` genera las capturas QA 1440×900 de ambos temas.
- AppStream: metainfo sin `developer_name` (queda `<developer><name>`), sin warnings de
  `appstreamcli validate`.
- `packaging/check-package.sh`: puerta de empaquetado que valida el control, los diez archivos
  instalados y las copias instaladas del `.desktop` y el metainfo, y ejecuta `lintian` si existe.

### Logging e i18n

- Logging estructurado con `tracing` a stderr (journald lo captura), filtro `RUST_LOG` o
  `SEGMENTA_LOG` con default `info`, campos de ciclo de vida (`task_id`, `attempt_id`, `state`,
  `status`, `error_code`, `retry`, `duration_ms`) y redacción estricta: URLs reducidas a
  `scheme://host/path`, sin cabeceras `Authorization`/`Cookie`, sin cuerpos, credenciales ni
  secretos. `journalctl --user -u segmenta` permite reconstruir un alta, una pausa, una
  reanudación y un fallo.
- Interfaz en español, portugués de Brasil e inglés según `LC_ALL`/`LC_MESSAGES`/`LANG`, con
  fallback a inglés. El protocolo JSON-RPC no cambia: los `code` siguen siendo la fuente y el
  texto se resuelve en el cliente.

## Instalación y actualización

Desde la release (junto al paquete viaja el `.sha256`):

```bash
sha256sum -c segmenta_0.0.2-1_amd64.deb.sha256
sudo apt install ./segmenta_0.0.2-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
segmenta system status   # secret_backend debe ser keyring
```

`apt install ./archivo.deb` resuelve la actualización sobre la 0.0.1 e instala
`libsecret-tools` como dependencia nueva; `apt upgrade` por sí solo no ve el archivo local.

Verificación rápida:

```bash
segmenta --version
segmenta-gui --version
dpkg-deb -f segmenta_0.0.2-1_amd64.deb Version
journalctl --user -u segmenta -n 50 --no-pager
```

## Notas de migración

- **Secretos (`secrets.json` → keyring).** Si existía `$XDG_DATA_HOME/segmenta/secrets.json` con
  entradas, al primer arranque con el keyring disponible se importan al Secret Service (una entrada
  por alias) y el archivo se borra. Si el keyring no está disponible, el archivo **no se usa ni se
  toca** y el backend se informa como `unavailable`: en ese caso, instalá `libsecret-tools`,
  asegurate de que el keyring del escritorio esté desbloqueado y volvé a arrancar el daemon.
- **`download_dir`.** Si el valor persistido era exactamente el default antiguo
  (`$HOME/Downloads`), el usuario no lo había elegido en `~/.config/user-dirs.dirs` y XDG resuelve
  otra carpeta, se migra una sola vez al valor XDG y queda registrado en el log. Un valor
  personalizado nunca se modifica. Revisá el valor con
  `segmenta settings get download_dir` y cambialo con
  `segmenta settings set download_dir "$HOME/Descargas"` si hace falta.
- **Socket de usuario.** El paquete no habilita la unidad desde `postinst`: el paso
  `systemctl --user enable --now segmenta.socket` sigue siendo del operador (la GUI ahora lo
  recuerda y permite reintentar).
- **Tareas de la 0.0.1.** La base SQLite, el DDL y el contrato IPC son compatibles; una tarea en
  estado terminal ya no se puede pausar ni reanudar aunque la CLI lo intentara en la 0.0.1.

## Limitaciones conocidas (heredadas de 0.0.1)

- Sólo descargas directas: **sin HLS/DASH, sin DRM, sin subtítulos ni multi-audio**.
- **Sin extensión de navegador** ni Native Messaging.
- `queue.schedule` persiste la configuración pero **no arranca ni detiene tareas por horario**, y
  `max_active` todavía no limita la concurrencia.
- Si un servidor ignora `Accept-Encoding: identity` y comprime, la descarga falla con
  `ERR_UNSUPPORTED` en lugar de escribir bytes comprimidos.
- `.deb` para Ubuntu 26.04 es el único formato de esta release (sin Flatpak/Snap/AppImage).
- Sin display no hay GUI: por SSH sólo se prueba con `xvfb-run -a segmenta-gui --smoke`.

## Estado del QA

El paquete está validado en local con `packaging/check-package.sh` (`dpkg-deb --info`,
`desktop-file-validate`, `appstreamcli validate --no-net` y `lintian` si está instalado) y con la
suite de tests del workspace. **La validación remota de este `.deb` está pendiente**: falta
instalarlo en `ssh ubuntu`, confirmar `secret_backend=keyring`, la carpeta `~/Descargas`, la
descarga segmentada con pausa/reanudación y SHA-256, la descarga HTTPS, `segmenta-gui --smoke`
bajo Xvfb, los tres idiomas y los logs en `journalctl`. Hasta ese informe el tag `v0.0.2` y la
publicación quedan sin efecto.

## Checksum

Ver `segmenta_0.0.2-1_amd64.deb.sha256` junto al paquete.
