# QA E2E remoto — Segmenta 0.0.1

Fecha: 2026-11-11  
Host: `ssh ubuntu` (`10.50.0.3`), Ubuntu 26.04.1 LTS, usuario `wilkin`.  
Artefacto: `segmenta_0.0.1-1_amd64.deb` del Release `v0.0.1`.  
SHA-256 del `.deb`: `8e3557dd255a285df6e390887c8eb14e3dd5a3182f10dacdaac3534c1bea0e06`.

## 1. Instalación y estado del paquete

- Descarga desde GitHub Release y `sha256sum -c`: **OK**.
- `sudo apt install ./segmenta_0.0.1-1_amd64.deb`: **OK**.
- Binarios instalados: `/usr/bin/segmenta`, `segmenta-cli`, `segmenta-daemon`, `segmenta-gui`.
- `segmenta.socket` y `segmenta.service` de usuario: **enabled y active** tras habilitarlos manualmente.
- `systemd` arranca el daemon por socket activation; `segmenta hello` responde `protocol_version=1`, `daemon_version=0.0.1`.
- `segmenta-gui --smoke` bajo `xvfb-run`: **SMOKE OK rows=1 segments=4**.
- `desktop-file-validate`: sin errores.
- `appstreamcli validate`: válido, con warning `developer-name-tag-deprecated`.
- `lintian` no está instalado en el host: no se ejecutó.

## 2. Pruebas E2E realizadas

### Descarga segmentada, pausa y reanudación

Servidor local con rangos HTTP, 128 MiB, throttling 2 MB/s por conexión, 8 conexiones.

- Se descargaron 29.229.056 bytes antes de pausar.
- `segmenta pause`: estado `paused`, segmentos `idle`, offsets conservados.
- `segmenta resume`: reanudó desde los offsets publicados.
- Al completar: `134.217.728` bytes, estado `completed`.
- SHA-256 del archivo descargado == SHA-256 de la fuente: **OK**.
- `cmp`: **OK**.

### Descarga HTTPS real

- Asset de GitHub `RELEASE_NOTES.md` (4.131 bytes): `completed`, contenido y SHA-256 **OK**.
- `speed.hetzner.de`: falló por DNS del entorno (`No address associated with hostname`), no por Segmenta; `curl` falla igual.
- SSRF: `127.0.0.1` queda bloqueado por defecto con `ERR_BLOCKED_PRIVATE_IP`; se habilitó `allow_local_downloads=true` sólo para el test y se restauró a `false`.

### Integración

- `.desktop`, icono y metainfo instalados.
- `systemctl --user` funciona; socket `0600`, directorio `0700`.
- GUI sólo se pudo probar headless; no hay sesión gráfica por SSH.

## 3. Defectos encontrados

### P1 — El backend de secretos nunca elige keyring

El host tiene `gnome-keyring-daemon` activo. Instalé `libsecret-tools` y `secret-tool` funciona
(store/lookup/clear), pero el daemon sigue reportando:

```json
{"secret_backend":"file"}
```

Causa: `KeyringSecretStore::available()` ejecuta `secret-tool --version` y exige exit code 0; esta
versión de `secret-tool` no soporta `--version` y devuelve 2. El fallback `file` se elige siempre.

Corrección esperada:

- Detectar disponibilidad sin `--version`: intentar `secret-tool lookup` o comprobar el ejecutable y
  tratar “sin secreto” como disponible; nunca usar un comando que no exista en libsecret.
- Añadir `libsecret-tools` como dependencia del `.deb` (`Depends` o `Recommends` + detección).
- Test de integración con `secret-tool` real y con un backend simulado sin él.
- Decidir/documentar si el fallback `file` queda permitido; hoy contradice `OpenSpec 11` (“nunca SQLite
  en claro” y keyring obligatorio).

### P1 — Directorio de descargas por defecto ignora XDG

En esta Ubuntu española:

```text
xdg-user-dir DOWNLOAD -> /home/wilkin/Descargas
settings.download_dir -> /home/wilkin/Downloads
```

Un `segmenta add` sin `-d` creó `/home/wilkin/Downloads` y descargó allí. Es un bug de integración y
localización. Corregir `default_download_dir()` para usar, en orden:

1. `XDG_DOWNLOAD_DIR` si está definido;
2. `xdg-user-dir DOWNLOAD` o el contenido de `~/.config/user-dirs.dirs`;
3. fallback `$HOME/Downloads` sólo si no hay configuración XDG.

Añadir test con `HOME` temporal y `user-dirs.dirs`, y migrar el valor viejo si es exactamente el
default histórico y el usuario nunca lo cambió (o dejarlo explícito en release notes).

### P1 — `pause`/`resume` aceptan tareas fallidas y devuelven éxito falso

Con una tarea `failed` por SSRF:

- `segmenta pause <id>` respondió `paused`, pero `status` siguió `failed`.
- `segmenta resume <id>` respondió `downloading`, pero `status` siguió `failed`.

Corrección: validar estados terminales (`failed`, `completed`, `cancelled`) y responder errores
semánticos (`TASK_NOT_PAUSABLE`, `TASK_NOT_RESUMABLE`), sin cambiar estado ni publicar un evento
engañoso. Añadir tests de las cuatro combinaciones.

### P2 — `speed_bps` queda obsoleto al pausar

El status pausado conservó `speed_bps: 16803851`; los segmentos sí quedaron con `speed_bps: 0`.
Resetear velocidad/ETA en `pause` y en cualquier parada, y testearlo.

### P2 — Dependencias e integración del paquete

- `libsecret-tools` no está en `Depends`; sin él se usa el fallback `file` silenciosamente.
- El paquete no habilita `segmenta.socket`: tras instalar, la GUI no funciona hasta ejecutar
  `systemctl --user enable --now segmenta.socket`. Decidir si el `postinst`/GUI puede hacerlo de forma
  segura o si se documenta como paso obligatorio.
- AppStream: eliminar `developer_name` (deprecado) y dejar sólo `<developer><name>`.
- `segmenta-gui --version` responde “Opción desconocida”; agregar `--version`/`about` o documentar que
  la versión se consulta en la GUI.

### P3 — Detalles de contrato/observación

- `segmenta list` devuelve `segments: []` para tareas completadas, mientras `status` devuelve la tabla
  final. Es coherente con un resumen, pero debe quedar explícito en el contrato.
- `secret_backend: file` conviene exponerlo también en `settings get` o en la GUI, no sólo en
  `system status`, para que el operador sepa dónde están sus secretos.
- `lintian` no se pudo ejecutar; añadirlo a la puerta de release cuando esté disponible.

## 4. Lo que ya funciona bien

- El motor segmentado respeta rangos, ETag/If-Range y checksums; pausa/reanudación no corrompe bytes.
- La reanudación conserva offsets y reconstruye la tabla final de segmentos.
- El daemon sobrevive y el socket systemd funciona.
- La GUI smoke test abre el modelo real con una fila y cuatro segmentos.
- El paquete se instala limpio y los binarios quedan en `/usr/bin`.
- La limpieza de tareas/archivos de prueba y el reset de `allow_local_downloads` quedaron hechos.

## 5. Criterios para 0.0.2

1. Backend `keyring` seleccionado cuando `secret-tool` está disponible; `.deb` depende de
   `libsecret-tools`; test con y sin backend.
2. `download_dir` respeta XDG/`Descargas`; test de locale y de `HOME` limpio.
3. `pause`/`resume` rechazan estados terminales con códigos semánticos y tests.
4. `speed_bps`/ETA se resetean al pausar.
5. AppStream sin warnings deprecados y `segmenta-gui --version` disponible.
6. Instalación limpia en Ubuntu 26.04 + smoke GUI + descarga segmentada + pausa/reanudación + HTTPS.
7. `.deb` 0.0.2 publicado en GitHub Release con SHA-256 y notas de migración.
8. `cargo fmt`, `clippy -D warnings`, `cargo test --workspace`, GUI headless y `dpkg-deb` verdes.
9. Repo limpio, sin secretos, `target/` y temporales eliminados; documentación actualizada.
