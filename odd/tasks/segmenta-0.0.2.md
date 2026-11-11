# Segmenta 0.0.2 — correcciones del QA remoto

Release `v0.0.2`: corrige los defectos P0/P1/P2 encontrados al probar `0.0.1` en `ssh ubuntu`
(Ubuntu 26.04.1 LTS), publica el paquete corregido y deja el remoto instalado y verificado. Documento
ODD; se lee hasta `## Log`. Alcance cerrado: **no** entra HLS/DASH, extensión de navegador ni Native
Messaging.

## Specs

### S1 — Detección real de keyring
`KeyringSecretStore` se considera disponible cuando el ejecutable `secret-tool` existe y una consulta
de sondeo termina con código 0 (secreto encontrado) o 1 (no hay secreto). Está prohibido usar
`secret-tool --version`, que en libsecret 0.21 devuelve 2. Criterio: `system.status.secret_backend`
responde `keyring` en un host con `secret-tool` funcional y `unavailable` cuando no lo hay.

### S2 — Sin fallback silencioso a archivo
El backend por defecto es keyring o «no disponible»; nunca se elige `FileSecretStore` de forma
automática. Con el backend no disponible:
- `download.add` que incluya `Authorization`, `Cookie` o `cookies[]` devuelve `1031`
  (`ERR_KEYRING_UNAVAILABLE`) y **no** crea la tarea (no se pierde el secreto en silencio).
- Si el secreto de una tarea existente deja de poder leerse, la tarea pasa a `paused` con `1031`.
Criterio: test con backend simulado ausente y test de `download.add` con cabecera sensible.

### S3 — Migración del archivo legado
Al arrancar con keyring disponible, si existe `$XDG_DATA_HOME/segmenta/secrets.json` con entradas,
se importan al keyring (una por alias) y el archivo se elimina; si el keyring no está disponible el
archivo se deja intacto, no se usa y se informa `unavailable`. Criterio: test con archivo legado y
backend de prueba.

### S4 — Directorio de descargas XDG
`default_download_dir()` resuelve en este orden: `XDG_DOWNLOAD_DIR` absoluto; salida de
`xdg-user-dir DOWNLOAD`; `XDG_DOWNLOAD_DIR` de `~/.config/user-dirs.dirs`; `$HOME/Downloads`. Nunca
hardcodea `Downloads`. Criterio: tests con `HOME` temporal sin y con `user-dirs.dirs`, con
`XDG_DOWNLOAD_DIR` y con `xdg-user-dir` en `PATH`; en el remoto, `segmenta add` sin `-d` descarga en
`/home/wilkin/Descargas`.

### S5 — Migración de `download_dir` histórico
Si el valor persistido es exactamente el default antiguo (`$HOME/Downloads`), no existe en
`user-dirs.dirs` como carpeta elegida y la resolución XDG da otra ruta, se migra al valor XDG una vez
y se registra en el log. Un valor personalizado nunca se toca. Criterio: test de migración y de no
migración con valor propio.

### S6 — `pause` sólo en estados pausables
`pause` se acepta en `downloading`, `pending`, `probing`, `retrying`, `verifying` y `finalizing`.
En `paused`, `completed`, `failed` y `cancelled` devuelve `1011` (`TASK_NOT_PAUSABLE`), no cambia el
estado y no publica eventos. Criterio: test de las combinaciones, incluida una tarea `failed` por
SSRF.

### S7 — `resume` sólo desde estados reanudables
`resume` se acepta en `paused` y `retrying`. En `completed`, `failed` y `cancelled` devuelve `1003`
(`TASK_NOT_RESUMABLE`) sin tocar el estado ni publicar `state_changed`. Criterio: test de las
combinaciones y del código JSON-RPC exacto.

### S8 — Velocidad y ETA en cero al pausar
Tras `pause` (o cualquier parada) el `status` de la tarea reporta `speed_bps = 0` y
`eta_seconds = null`; los segmentos ya lo hacían. Criterio: test de `download.status` pausado y de
una pausa real con motor en marcha.

### S9 — Paquete: `libsecret-tools`
El `.deb` declara `libsecret-tools` en `Depends` y `dpkg-deb --info` lo muestra. Criterio: inspección
del control del paquete generado.

### S10 — AppStream sin deprecaciones
El metainfo usa sólo `<developer><name>` (sin `developer_name`) y `appstreamcli validate` no reporta
warnings. Criterio: `appstreamcli validate --no-net` sin `developer-name-tag-deprecated`.

### S11 — `segmenta-gui --version` y arranque
`segmenta-gui --version` imprime `segmenta-gui 0.0.2` y sale 0; `--help` documenta `--smoke`. Cuando
el socket no está activo, la GUI muestra un estado de error accionable con el comando
`systemctl --user enable --now segmenta.socket` y un botón de reintento. Criterio: test del flag y del
texto del estado de error.

### S12 — Contrato de `list` vs `status`
`download.list` devuelve `segments: []` (resumen); `download.status` incluye la tabla de segmentos.
`system.status` incluye `secret_backend`. Ambas cosas quedan escritas en OpenSpec 07. Criterio: doc
actualizada y tests existentes alineados.

### S13 — Puerta de empaquetado
`packaging/check-package.sh` valida un `.deb` con `dpkg-deb --info`, `desktop-file-validate` y
`appstreamcli validate`, y ejecuta `lintian` si está disponible (si no, lo informa sin fallar).
Criterio: el script pasa sobre el `.deb` de 0.0.2 y falla si falta un binario o un archivo esperado.

### S14 — Versión 0.0.2 coherente
Workspace, binarios (`--version`), metainfo (entrada `<release version="0.0.2">`), `CHANGELOG.md` y
notas de release dicen 0.0.2. Criterio: `segmenta --version`, `segmenta-gui --version` y
`dpkg-deb -f … Version` coinciden.

### S15 — Release 0.0.2
Tag anotado `v0.0.2` sobre `main` verde y GitHub Release con `.deb`, `.sha256` y notas de migración;
el Release 0.0.1 queda intacto. Criterio: `gh release view v0.0.2` lista los assets y el checksum
verifica contra el archivo descargado.

### S16 — Verificación en el remoto
En `ssh ubuntu`: actualizar el paquete, `systemctl --user daemon-reload` + `enable --now
segmenta.socket`, `segmenta hello`/`system status` con `secret_backend=keyring`, descarga local
segmentada con pausa/reanudación y SHA-256, descarga HTTPS real, `segmenta-gui --smoke` bajo
`xvfb-run`, `segmenta add` sin `-d` en `~/Descargas`, y `pause`/`resume` sobre una tarea `failed`
fallando con código. Criterio: salida registrada de cada paso.

### S17 — Limpieza
Sin secretos de prueba en el keyring, sin tareas ni archivos de prueba, `allow_local_downloads=false`
y `download_dir` correcto en el remoto; `target/`, `dist/` y temporales eliminados en local cuando la
release está publicada. Criterio: `secret-tool lookup service segmenta key probe` vacío,
`segmenta list` vacío y árbol local limpio.

### S18 — Higiene de repositorio
Sin secretos, `.env`, `target/` ni `.atl/` versionados; árbol limpio al cerrar. Criterio:
`git status --porcelain` vacío y `git ls-files` sin artefactos.

### S19 — Logging estructurado y auditable
El daemon usa `tracing` + `tracing-subscriber` con salida a stderr (journald la captura) y filtro
`RUST_LOG`/`SEGMENTA_LOG` (default `info`). Cada evento de ciclo de vida lleva `task_id`,
`attempt_id`, `state`, `status`, `error_code`, `retry` y `duration_ms` cuando aplica. La GUI loguea en
stderr el fallo de conexión con el comando accionable. Criterio: `journalctl --user -u segmenta`
permite reconstruir un add, una pausa, una reanudación y un fallo; hay test de niveles (`info` no
emite `debug`) y de que un nivel inválido cae en `info`.

### S20 — Redacción estricta en logs
Nunca se registran `Authorization`, `Cookie`, `Set-Cookie`, cuerpos, credenciales, secretos del
keyring ni query strings con tokens. Las URLs se reducen a `scheme://host/path` (sin query ni
fragment) y los valores sensibles se reemplazan por `[redacted]`. Criterio: tests que loguean una
URL con `?token=...`, una cabecera `Authorization: Bearer ...`, un par cookie=valor y un JSON con
`"hash"`/`"password"` y verifican que el texto final no contiene el secreto y sí `[redacted]`.

### S21 — i18n es / pt-BR / en
Crate `segmenta-i18n` sin dependencias, con catálogos completos y detección por `LC_ALL`,
`LC_MESSAGES`, `LANG` en ese orden, normalizando `es_*`→`es`, `pt_BR*`→`pt-BR`, `en_*`→`en` y
`C`/`POSIX`/desconocido→`en`. Toda cadena visible de la GUI (ventana, diálogos, preferencias,
estados, errores, tooltips y textos accesibles) y de la CLI/errores humanos del daemon sale del
catálogo. El protocolo JSON-RPC no cambia: los `code` siguen siendo la fuente y el texto se resuelve
en el cliente. Criterio: tests de detección de locale, de paridad de claves entre los tres catálogos
y de que no queda texto visible hardcodeado en la GUI.

### S22 — Tema claro/oscuro real
La apariencia sigue `AdwStyleManager` y una preferencia nueva `appearance` (`system`|`light`|`dark`,
default `system`) en el catálogo de settings; la UI no fija colores que rompan contraste y la barra de
segmentos usa el color de acento del tema. Criterio: capturas 1440×900 de ambos temas generadas por
`segmenta-gui --screenshots <dir>` (nunca a mano), revisión de contraste/foco/selección en ambos y
test de que `--screenshots` escribe `segmenta-light.png` y `segmenta-dark.png` no vacíos.

### S23 — Arranque normal de la GUI
Con el daemon activo y una tarea completada, `segmenta-gui` debe listarla en menos de 2 s. Si el socket
no está disponible, la UI muestra un estado de error accionable con el comando
`systemctl --user enable --now segmenta.socket`, el motivo y un botón **Reintentar** que reconecta de
verdad (reintenta la conexión y recarga la lista). Criterio: test de integración con un `UnixListener`
real que responde `download.list` y otro con un socket ausente que exige el comando y ejercita el
reintento hasta que el daemon aparece.

### S24 — Todos los controles funcionales
Ningún control decorativo: cada botón, ítem de menú, fila de la barra lateral, atajo y acción de la
matriz de controles tiene handler real, estado deshabilitado explicado, feedback visual y etiqueta
accesible, con test. Si un control no se puede completar en 0.0.2, se quita de la UI y se documenta la
decisión. Criterio: la matriz de controles de este documento queda cubierta por tests de acción (no
sólo de renderizado).

### S25 — Capturas y validación visual
Las capturas viven en `docs/qa/screenshots/` con nombres estables (`0.0.2-light.png`,
`0.0.2-dark.png`, `0.0.1-dark.png` histórico) y se generan con el modo `--screenshots`, no con
herramientas externas. Criterio: las dos capturas existen, pesan más de 10 KB, son 1440×900 y se
revisan contraste, foco, selección, segmentos y estados de error en ambas.

## Tasks

| ID | Specs | Tarea | Rutas | Test RED |
| --- | --- | --- | --- | --- |
| T1 | S1-S3 | Detección de keyring sin `--version`, backend «no disponible» y migración del archivo legado | `crates/segmenta-daemon/src/secrets.rs`, `manager.rs` | `keyring_detection_uses_lookup_not_version`, `add_with_sensitive_headers_requires_a_backend`, `legacy_file_is_migrated` |
| T2 | S4-S5 | Resolución XDG y migración del default histórico | `store.rs`, `manager.rs` | `download_dir_prefers_xdg_user_dir`, `download_dir_migrates_the_historical_default` |
| T3 | S6-S8 | Validación de estados en pause/resume y reset de velocidad | `manager.rs`, `rpc.rs` | `pause_rejects_terminal_states`, `resume_rejects_terminal_states`, `paused_status_reports_zero_speed` |
| T4 | S9-S11 | Paquete, AppStream y flags de la GUI | `crates/segmenta-gui/{Cargo.toml,src/main.rs,src/window.rs}`, `packaging/*.xml` | `gui_version_flag_prints_the_version`, `missing_socket_shows_the_enable_command` |
| T5 | S12-S13 | Contrato de list/status, puerta de empaquetado y CHANGELOG | `docs/openspec/07`, `packaging/check-package.sh`, `CHANGELOG.md` | script sobre un `.deb` real |
| T6 | S14 | Bump a 0.0.2 y metainfo | `Cargo.toml`, `crates/segmenta-gui/data/*.xml`, `packaging/` | `--version` y `dpkg-deb -f` |
| T7 | S15-S17 | Release, instalación remota y limpieza | `dist/`, remoto | comandos de S16 |
| T8 | S19-S20 | Logging con `tracing`, filtros y redacción estricta | `crates/segmenta-daemon/Cargo.toml`, `src/logging.rs`, `manager.rs`, `ipc.rs`, `main.rs` | `logs_redact_urls_and_headers`, `log_level_defaults_to_info` |
| T9 | S21 | Crate `segmenta-i18n`, catálogos es/pt-BR/en y detección de locale | `crates/segmenta-i18n/**`, `crates/segmenta-cli/**` | `locale_detection_*`, `catalogs_have_the_same_keys`, `no_hardcoded_visible_strings` |
| T10 | S22, S25 | Apariencia `system/light/dark`, estilo y modo capturas | `crates/segmenta-gui/src/{app,window,prefs}.rs`, `main.rs` | `screenshots_are_written_for_both_themes` |
| T11 | S23 | Arranque normal, error accionable y reintento real | `crates/segmenta-gui/src/{client,window}.rs` | `normal_startup_lists_a_completed_task`, `retry_reconnects_after_the_socket_appears` |
| T12 | S24 | Matriz de controles: handlers, estados y tests de acción | `crates/segmenta-gui/src/**`, `tests/actions.rs` | `button_matrix_is_covered` |
| T13 | S19-S25 | Documentación, capturas y puerta de release sin publicar | `README.md`, `docs/openspec/*`, `CHANGELOG.md`, `docs/qa/**` | `appstreamcli`/`desktop-file-validate` |

## Riesgos, rollback y presupuesto

- **Riesgo**: quitar el fallback a archivo puede dejar sin secretos a un usuario que los tenía en
  `secrets.json`. Mitigación: importación automática al keyring con borrado del archivo y, si el
  keyring no está, no se toca ni se usa, y el error es explícito (S2/S3).
- **Riesgo**: la migración de `download_dir` podría pisar una preferencia del usuario. Mitigación:
  sólo se migra el default histórico exacto y sólo si la carpeta antigua no fue elegida en
  `user-dirs.dirs` (S5).
- **Riesgo**: tests de keyring dependen del entorno. Mitigación: backend simulado obligatorio en la
  suite; la prueba real se marca como *manual* y se ejecuta en el remoto (S16).
- **Rollback**: cada tarea es un commit atómico; `git revert <sha>`. El tag se crea al final; si una
  puerta falla no se etiqueta.
- **Presupuesto**: 3–4 h de agente; si el remoto no responde, se entrega local + release y se informa
  el bloqueo exacto sin inventar la verificación remota.

## Matriz de pruebas

| Capa | Herramienta | Casos |
| --- | --- | --- |
| Secretos | `cargo test -p segmenta-daemon` | detección por lookup, backend ausente, migración del archivo legado |
| Settings/XDG | `cargo test -p segmenta-daemon` | `XDG_DOWNLOAD_DIR`, `user-dirs.dirs`, `xdg-user-dir`, migración y no migración |
| Máquina de estados | `cargo test -p segmenta-daemon --test ipc` / unitarias del manager | pause/resume por estado, códigos 1011/1003, sin eventos |
| GUI | `xvfb-run -a cargo test -p segmenta-gui` | `--version`, estado de error con comando y reintento |
| Empaquetado | `packaging/check-package.sh` | `dpkg-deb --info`, `desktop-file-validate`, `appstreamcli validate`, lintian si existe |
| Logging | `cargo test -p segmenta-daemon` | redacción de URL/cabeceras/cookies/cuerpos, niveles, campos de ciclo de vida |
| i18n | `cargo test -p segmenta-i18n` + `-p segmenta-cli` | detección es/pt-BR/en/desconocido, paridad de claves, textos de CLI y errores |
| GUI arranque | `xvfb-run -a cargo test -p segmenta-gui` | daemon real con tarea completada <2 s, socket ausente con comando + reintento que reconecta |
| GUI controles | `xvfb-run -a cargo test -p segmenta-gui --test actions` | matriz de botones: cada control dispara su handler, estados deshabilitados y confirmaciones |
| Capturas | `xvfb-run -a segmenta-gui --screenshots docs/qa/screenshots` | `0.0.2-light.png` y `0.0.2-dark.png` 1440×900 |
| Remoto | `ssh ubuntu` | instalación, keyring, XDG, descarga 128 MiB + pausa + SHA-256, HTTPS, smoke, pause/resume sobre `failed` |

## Comandos exactos

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
xvfb-run -a cargo test -p segmenta-gui

cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.0.2-1_amd64.deb
(cd dist && sha256sum segmenta_0.0.2-1_amd64.deb > segmenta_0.0.2-1_amd64.deb.sha256)

# remoto
scp dist/segmenta_0.0.2-1_amd64.deb ubuntu:/tmp/
ssh ubuntu 'sudo apt install -y /tmp/segmenta_0.0.2-1_amd64.deb && systemctl --user daemon-reload && systemctl --user enable --now segmenta.socket && segmenta system status'

# release
git tag -a v0.0.2 -m "Segmenta 0.0.2"
gh release create v0.0.2 dist/segmenta_0.0.2-1_amd64.deb dist/segmenta_0.0.2-1_amd64.deb.sha256 \
  --title "Segmenta 0.0.2" --notes-file dist/RELEASE_NOTES_0.0.2.md
```

## Matriz de controles (S24)

Cada fila necesita handler real, estado deshabilitado explicado cuando corresponda, feedback visual,
etiqueta/tooltip accesible y un test de acción.

| Control | Acción esperada | Estados | Test |
| --- | --- | --- | --- |
| Sidebar Todas/Activas/Pausadas/Completadas/Fallidas | filtra la lista y actualiza contadores reales | siempre activo | `filter_counts_and_rows_match` |
| `+ Añadir URL` / `Ctrl+N` | abre el diálogo de alta | deshabilitado mientras no hay cliente | `add_dialog_opens_and_cancels` |
| Diálogo: Elegir carpeta | `GtkFileDialog` y refleja la ruta elegida | activo | `add_dialog_picks_a_folder` |
| Diálogo: Añadir | valida URL/carpeta/nombre, llama `download.add`, cierra y muestra toast | deshabilitado si la URL está vacía, con motivo | `add_dialog_submits_and_reports_errors` |
| Diálogo: Cancelar | cierra sin llamar al daemon | activo | `add_dialog_cancel_sends_nothing` |
| Menú hamburguesa | Preferencias, Recargar, Acerca de | activo | `menu_actions_fire` |
| Preferencias / `Ctrl+,` | abre; cada fila guarda con `settings.update`; error en toast | activo | `preferences_rows_persist` |
| Buscar / `Ctrl+F` | abre/cierra la barra, filtra en vivo, limpiar y estado vacío | activo | `search_filters_and_clears` |
| Fila de la lista | abre el detalle de esa tarea | activo | `row_selection_opens_detail` |
| Pausar | `download.pause` y refresca estado | visible sólo si `downloading` | `detail_actions_follow_state` |
| Reanudar | `download.resume` | visible sólo si `paused` | `detail_actions_follow_state` |
| Cancelar | confirma y `download.cancel` | visible si no es terminal | `destructive_actions_confirm` |
| Eliminar | confirma y `download.delete` | visible si es terminal o pausada | `destructive_actions_confirm` |
| Abrir carpeta | `xdg-open` sobre el directorio del archivo | visible si hay ruta | `open_folder_uses_the_parent_directory` |
| Reintentar (error) | reconecta el cliente y recarga la lista | visible sólo en error | `retry_reconnects_after_the_socket_appears` |
| Acerca de | `AdwAboutDialog` con versión y licencia | activo | `about_dialog_opens` |
| Recargar | `download.list` inmediato | activo | `reload_fetches_again` |
| Quit / `Ctrl+Q` | cierra la ventana y termina | activo | `quit_closes_the_window` |
| Controles de ventana | minimizar/maximizar/cerrar nativos | según el gestor | smoke de widgets |

## Criterio de release

`v0.0.2` se etiqueta sólo si: las cuatro puertas locales pasan; la GUI normal lista una tarea
completada en menos de 2 s; existen capturas light/dark revisadas; `LANG=es_ES`, `LANG=pt_BR.UTF-8` y
`LANG=en_US.UTF-8` muestran la UI en su idioma; `journalctl --user -u segmenta` reconstruye un add,
una pausa, una reanudación y un fallo sin secretos; la matriz de controles está cubierta por tests de
acción; el `.deb` valida con
`check-package.sh`; en el remoto el backend es `keyring`, `segmenta add` sin `-d` cae en
`~/Descargas`, una descarga segmentada de 128 MiB con pausa y reanudación conserva el SHA-256, la
descarga HTTPS funciona, el smoke de la GUI pasa y `pause`/`resume` sobre una tarea `failed` fallan
con código; el remoto queda limpio con `allow_local_downloads=false`; el repo queda limpio y el
Release 0.0.2 publica `.deb`, `.sha256` y notas sin tocar el 0.0.1.

## Log

**L1 (solicitud original del usuario, verbatim):**

> Tomá el frente Segmenta 0.0.2 en `/home/wilkin/proyectos/Segmenta`, sobre `main`, sin ramas ni worktrees. Acabamos de instalar y probar `0.0.1` en el Ubuntu remoto (`ssh ubuntu`, Ubuntu 26.04.1 LTS). El informe completo está en `docs/qa/segmenta-0.0.1-e2e.md`; leelo completo antes de tocar código.
>
> El objetivo es corregir los defectos reales encontrados, publicar `0.0.2`, actualizar la documentación, limpiar espacio y dejar el paquete instalado/probado en el Ubuntu remoto. No cambies el alcance a HLS/DASH, extensión ni Native Messaging; eso sigue fuera de esta release.
>
> ## Contexto de la prueba remota
>
> - `.deb` 0.0.1 instalado desde GitHub Release; SHA-256 correcto.
> - `segmenta.socket`/`segmenta.service` activos; CLI, GUI smoke, `.desktop` y AppStream validados.
> - Descarga local de 128 MiB con 8 conexiones: pausa, reanudación y SHA-256 idéntico.
> - Descarga HTTPS real de GitHub: OK.
> - SSRF bloqueó `127.0.0.1` por defecto; se habilitó `allow_local_downloads` para el test y se restauró a `false`.
> - El host tiene `gnome-keyring-daemon` y `libsecret-tools` instalado; `secret-tool store/lookup/clear` funciona.
> - `segmenta system status` sigue diciendo `secret_backend=file`.
> - `xdg-user-dir DOWNLOAD` es `/home/wilkin/Descargas`, pero `settings.download_dir` era `/home/wilkin/Downloads`.
> - `segmenta pause`/`resume` sobre una tarea `failed` devolvieron éxito (`paused`/`downloading`) mientras el status seguía `failed`.
> - El status pausado conservó `speed_bps=16803851`; los segmentos sí quedaron en 0.
> - AppStream emite `developer-name-tag-deprecated`.
> - `segmenta-gui --version` responde “Opción desconocida”.
> - No hay display por SSH; la GUI sólo se prueba con `xvfb-run`.
> - La limpieza remota ya se hizo: sin tareas, sin `/tmp/segmenta-e2e`, `allow_local_downloads=false`, `download_dir=/home/wilkin/Descargas`.
>
> ## P0/P1 obligatorios de 0.0.2
>
> ### 1. Keyring real
>
> El bug está en `KeyringSecretStore::available()`: ejecuta `secret-tool --version`, que en esta versión devuelve exit 2, entonces siempre elige `FileSecretStore`.
>
> - Cambiá la detección para no depender de `--version`: comprobá el ejecutable y un `lookup` que devuelva “sin secreto” como backend disponible, o usá una API D-Bus correcta. No inventes un comando que libsecret no tenga.
> - Añadí `libsecret-tools` como dependencia/recomendación del `.deb` y verificá que el keyring se use cuando existe.
> - Si `secret-tool` no está o el keyring falla, el comportamiento debe ser explícito y documentado. Preferencia: tarea `paused` con `ERR_KEYRING_UNAVAILABLE` para secretos, no fallback silencioso a archivo; si conservás el fallback, no contradigas OpenSpec 11 y avisalo en `system.status`/GUI.
> - Tests: backend keyring real/manual y backend simulado sin `secret-tool`; verificar que `system.status` diga `keyring` en el host con `secret-tool`.
> - Verificá que no queden secretos de prueba en el keyring (`secret-tool clear ...`).
>
> ### 2. Directorio de descargas XDG
>
> `default_download_dir()` hardcodea `$HOME/Downloads`. En Ubuntu español el usuario tiene `~/Descargas`.
>
> - Resolver en este orden: `XDG_DOWNLOAD_DIR`; `xdg-user-dir DOWNLOAD` o `~/.config/user-dirs.dirs`; fallback `$HOME/Downloads`.
> - Test con `HOME` temporal, sin y con `user-dirs.dirs`, y con `XDG_DOWNLOAD_DIR`.
> - Migrar un valor antiguo sólo si coincide exactamente con el default histórico y el usuario no lo personalizó; documentar la migración.
> - Verificar en el remoto que una descarga sin `-d` va a `/home/wilkin/Descargas`.
>
> ### 3. Estados de pause/resume
>
> - `pause` sólo válido para `downloading`/`pending` (o el estado que corresponda); `resume` sólo para `paused`/`retrying`.
> - `failed`, `completed`, `cancelled` deben devolver errores semánticos (`TASK_NOT_PAUSABLE`, `TASK_NOT_RESUMABLE`), sin cambiar estado ni emitir eventos falsos.
> - Tests de las combinaciones y de una tarea fallida por SSRF.
> - No cambies el estado de una tarea terminal.
>
> ### 4. Velocidad al pausar
>
> - Al pausar/cancelar, `speed_bps` y `eta_seconds` deben quedar en 0/null también a nivel de tarea; los segmentos ya hacen lo correcto.
> - Test de status pausado.
>
> ### 5. Paquete e integración
>
> - Añadir `libsecret-tools` a dependencias/recomendaciones; verificar `dpkg-deb --info`.
> - Resolver el arranque del socket tras instalar: si no es seguro habilitarlo desde `postinst` como usuario, documentar claramente el paso `systemctl --user enable --now segmenta.socket` y hacer que la GUI muestre una acción/reintento si el socket no está activo. No ocultes el fallo.
> - AppStream: quitar `developer_name` deprecado y dejar `<developer><name>`; volver a validar con `appstreamcli`.
> - `segmenta-gui --version` debe funcionar o estar documentado; si es barato, agregalo.
> - Revisar `segmenta list` completado: `segments: []` puede ser correcto como resumen, pero documentá el contrato.
> - Añadir un test/chequeo de `desktop-file-validate` y AppStream a la puerta de release.
>
> ## TDD y documentación
>
> - Creá `odd/tasks/segmenta-0.0.2.md` antes de código, con specs, tasks, riesgos y criterios. Commiteá sólo el ODD primero.
> - Tests RED antes de cada fix; luego GREEN y refactor.
> - Actualizá `README.md`, `docs/openspec` afectados, `CHANGELOG.md` y las notas de release; documentá la migración de `download_dir` y el backend de secretos.
> - No agregues secretos, `.env`, `target/`, `.atl/` al repo.
> - Todo a `main`, commits atómicos, sin ramas.
>
> ## Validación y release 0.0.2
>
> - `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `xvfb-run -a cargo test -p segmenta-gui`, `dpkg-deb --info`, `desktop-file-validate`, `appstreamcli validate`.
> - Compilar release, generar `.deb` 0.0.2 + `.sha256`, instalar/actualizar en `ssh ubuntu`, habilitar socket, ejecutar:
>   - `segmenta hello`/`system status` (backend keyring),
>   - descarga segmentada desde servidor local con 8 conexiones, pausa, reanudación y SHA-256,
>   - descarga HTTPS real,
>   - `segmenta-gui --smoke` bajo Xvfb,
>   - una descarga sin `-d` que debe caer en `/home/wilkin/Descargas`,
>   - `pause`/`resume` sobre una tarea fallida debe fallar con código, no falsear éxito.
> - Limpiar datos de prueba remotos y locales, restaurar `allow_local_downloads=false`, no dejar `/tmp/segmenta-e2e`, borrar `target/` y artefactos intermedios, y liberar espacio.
> - Tag `v0.0.2` y GitHub Release con `.deb`, `.sha256` y notas; no borres el Release 0.0.1.
> - Verificar SHA del asset publicado y que `main` quede limpio/sincronizado.
>
> ## Cierre
>
> Reportá: defectos corregidos, tests, instalación remota 0.0.2, descarga/pausa/reanudación, backend keyring, XDG, AppStream, tag/release, limpieza y cualquier limitación real. Si algo no se puede probar por el entorno, decilo explícitamente; no inventes un resultado.

**L3 — Ampliación obligatoria antes de publicar.** El QA visual remoto
(`docs/qa/segmenta-0.0.2-ui-logging.md`) agrega cinco frentes: logging estructurado con redacción,
i18n es/pt-BR/en, tema claro/oscuro verificable, arranque normal de la GUI (la ventana quedaba en
«Conectando…» con una tarea completada) y una auditoría de todos los controles. Se agregan S19-S25,
T8-T13 y la matriz de controles. **No se publica `v0.0.2` hasta que el usuario instale el `.deb` en
`ssh ubuntu`, pruebe los controles, capture claro/oscuro y valide i18n/logs, y devuelva el informe.**

**L2 — Estado inicial.** `main` en `8df518a` (release 0.0.1) con `docs/qa/segmenta-0.0.1-e2e.md`
sin versionar. El remoto `ssh ubuntu` responde; tiene `secret-tool`, `xdg-user-dir`, `appstreamcli` y
`desktop-file-validate`; `xdg-user-dir DOWNLOAD` devuelve `/home/wilkin/Descargas` y
`secret-tool lookup service segmenta key probe` sale con 1 (sin secreto), que es exactamente la señal
de «backend disponible» que pide S1.
