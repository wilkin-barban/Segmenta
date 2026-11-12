# OpenSpec 10: Empaquetado, Ejecución Segura y Suite de Pruebas

> **Especificación:** OPENSPEC-SEGMENTA-010
> **Versión:** 2.0.0 (0.2.0 **en preparación**)
> **Área:** Configuración de Build Cargo, Empaquetado `.deb` para Ubuntu 26.04, Suite de Pruebas y Benchmarks
>
> **Alcance de entrega:** paquete `.deb` 0.2.0 y árbol `cargo` reproducible. Flatpak/Snap/AppImage
> quedan fuera de alcance; el Native Messaging fuera del sandbox no se resuelve con un manifiesto
> dentro del paquete y requiere una decisión de producto posterior.
>
> Todo lo marcado «(0.2, en preparación)» es contrato acordado para 0.2.0, no comportamiento entregado
> hasta que Q1 cierre con el QA real. No existe tag `v0.2.0` mientras esta marca siga aquí.

---

## 1. Definición del Workspace `Cargo.toml`

Rust, edición 2021, MSRV **1.80**. El workspace real tiene seis crates:

```toml
[workspace]
resolver = "2"
members = [
    "crates/segmenta-core",
    "crates/segmenta-i18n",
    "crates/segmenta-daemon",
    "crates/segmenta-cli",
    "crates/segmenta-gui",
    "crates/segmenta-host",
]

[workspace.package]
version = "0.2.0"          # Q1: hoy 0.1.2, se sube a 0.2.0 al cerrar la release
edition = "2021"
rust-version = "1.80"
license = "GPL-3.0-or-later"
repository = "https://github.com/wilkin-barban/Segmenta"
```

Cada crate hereda con `version.workspace = true`; no hay versiones por crate. Dependencias externas
relevantes que **no** están en `[workspace.dependencies]` y se fijan en su crate:

* `segmenta-gui`: `gtk4` 0.11 (feature `v4_10`) y `libadwaita` 0.9 (feature `v1_5`), más `glib` 0.22.
* `segmenta-daemon`: `librqbit` 9.1 y `librqbit-bencode` 9.1 para torrents, `zip` 0.6 para capturas.
* `segmenta-core`: `url` 2 y el resto por workspace (`reqwest` con `rustls-tls`).

`[workspace.dependencies]` contiene sólo lo compartido: `clap`, `futures-util`, `hex`, `httpdate`,
`libc`, `percent-encoding`, `rand`, `reqwest`, `rusqlite` (bundled), `serde`, `serde_json`, `sha2`,
`tempfile`, `thiserror`, `tokio`, `tracing`, `tracing-subscriber`, `uuid` y `segmenta-i18n`.

---

## 2. Empaquetado para Ubuntu 26.04 (`.deb`; Flatpak fuera de alcance)

### 2.0. Dependencias del sistema

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  libsecret-1-dev libsecret-tools libssl-dev ffmpeg
cargo install cargo-deb
```

`ffmpeg` es dependencia de ejecución para HLS/DASH (>= 7.0) y `yt-dlp` para el flujo de YouTube; los
declara `[package.metadata.deb].depends` en `crates/segmenta-gui/Cargo.toml`. La licencia de FFmpeg
debe revisarse antes de distribuir un binario que lo enlace. `libsecret` se usa vía D-Bus/Secret
Service; no se enlaza una biblioteca que guarde secretos en SQLite. CI fija Ubuntu 26.04 LTS.

`libsecret-tools` es **dependencia de ejecución declarada en el `.deb`** y `packaging/check-package.sh`
falla si falta (§2.2.1). El backend de secretos elige el keyring sólo cuando `secret-tool` existe y una
consulta de sondeo termina con código 0 (secreto encontrado) o 1 (no hay secreto); **nunca** se usa
`secret-tool --version`, que en libsecret 0.21 devuelve 2. Si el keyring no está disponible, el backend
se informa `unavailable`, no se escribe ningún secreto en archivo plano y un alta con cabeceras
sensibles falla con `1031` (`ERR_KEYRING_UNAVAILABLE`).

**ClamAV es opcional** (0.2, en preparación): no está en `Depends` ni se embebe. Se detecta
`clamdscan` primero y `clamscan` después; con `antivirus_enabled=true` y sin motor, la tarea falla con
`ERR_ANTIVIRUS_UNAVAILABLE` en vez de publicarse como `completed` (contrato en 12).

### 2.1. Contrato de instalación que valida la puerta (22 rutas)

`packaging/check-package.sh` compara `dpkg-deb --contents` contra esta lista exacta. Cualquier ruta
nueva o renombrada obliga a actualizar el script y esta tabla en el mismo commit.

```text
usr/bin/segmenta
usr/bin/segmenta-cli
usr/bin/segmenta-daemon
usr/bin/segmenta-gui
usr/bin/segmenta-host
etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json
etc/chromium/native-messaging-hosts/io.github.wilkin.segmenta.json
etc/opt/edge/native-messaging-hosts/io.github.wilkin.segmenta.json
etc/brave/native-messaging-hosts/io.github.wilkin.segmenta.json
etc/vivaldi/native-messaging-hosts/io.github.wilkin.segmenta.json
etc/opera/native-messaging-hosts/io.github.wilkin.segmenta.json
usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json
usr/lib/systemd/user/segmenta.service
usr/lib/systemd/user/segmenta.socket
usr/share/applications/io.github.wilkin.Segmenta.desktop
usr/share/applications/segmenta-handler.desktop
usr/share/metainfo/io.github.wilkin.Segmenta.metainfo.xml
usr/share/icons/hicolor/scalable/apps/io.github.wilkin.Segmenta.svg
usr/share/doc/segmenta/copyright
usr/share/segmenta/extensions/segmenta-chromium.zip
usr/share/segmenta/extensions/segmenta-firefox.zip
usr/share/segmenta/extensions/README
```

En la tabla, por función:

| Grupo | Rutas | Función |
| :--- | :--- | :--- |
| Binarios | `usr/bin/segmenta`, `segmenta-cli`, `segmenta-daemon`, `segmenta-gui`, `segmenta-host` | CLI corta, CLI, daemon, GUI y host de Native Messaging |
| Manifiestos nativos | seis rutas de navegador + `usr/lib/mozilla/native-messaging-hosts/` | Autorizan el host a nivel de sistema |
| systemd | `segmenta.service`, `segmenta.socket` | Daemon de usuario y activación por socket |
| Escritorio | `.desktop`, `segmenta-handler.desktop`, metainfo, icono | Menú, handler de magnet, App Center y GNOME Software |
| Extensión | los dos ZIP + `README` | Artefactos de carga manual de la extensión |
| Legal | `usr/share/doc/segmenta/copyright` | Licencia instalada |

`segmenta-handler.desktop` registra el handler de enlaces magnet; el `.deb` no ejecuta `xdg-mime`
desde `postinst`, así que el registro efectivo depende del escritorio y se documenta como tal.

### 2.2. Flujo de build e instalación (0.2, en preparación)

```bash
cargo build --release --workspace
node extension/tools/sync-shared.mjs
node extension/tools/pack.mjs
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0
sudo apt install ./dist/segmenta_0.2.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

`node extension/tools/pack.mjs` genera `dist/segmenta-extension-<versión>.zip` (Chromium MV3) y
`dist/segmenta-firefox-<versión>.zip`. Los `assets` de `[package.metadata.deb]` apuntan hoy a los
nombres `0.1.2`; Q1 debe actualizarlos a `0.2.0` **antes** de construir el paquete o `cargo deb` falla
buscando los ZIP.

El `.deb` no debe ejecutar `systemctl enable` desde `postinst` sin una decisión explícita: la unidad de
usuario se activa por el operador o por la GUI. El desinstalador conserva `$XDG_DATA_HOME/segmenta` y
pregunta antes de borrar descargas; el paquete no borra archivos del usuario.

### 2.2.1. Puerta de empaquetado (`packaging/check-package.sh`)

Es la puerta obligatoria entre `cargo deb` y cualquier tag o publicación. Se ejecuta sobre un `.deb`
ya construido —no lo genera— y valida el paquete real, no las fuentes:

```bash
packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0
```

Qué comprueba, en este orden y saliendo con código distinto de cero en el primer fallo duro:

1. que el archivo exista y sea un `.deb`, y que `dpkg-deb` esté instalado;
2. que `dpkg-deb --info` declare `Package: segmenta`, la `Version` esperada y un `Description` no
   vacío, y que `dpkg-deb -f Depends` incluya `libsecret-tools`. La versión esperada llega como
   segundo argumento (`0.2.0` en el gate) o por `SEGMENTA_EXPECTED_VERSION`; el valor por defecto del
   script es `0.1.2` y se sube junto con el workspace. Acepta `0.2.0` y `0.2.0-*`;
3. que `dpkg-deb --contents` liste las **22 rutas** de §2.1; informa **todas** las que falten, no sólo
   la primera;
4. que `desktop-file-validate` y `appstreamcli validate --no-net` acepten las copias **extraídas del
   propio paquete** con `dpkg-deb -x` en un directorio temporal de `mktemp -d` que se borra con `trap`
   al salir (nunca una ruta fija como `/tmp/...`); ambos validadores son requisitos del gate;
5. `lintian` si está instalado. Es **opcional e informativo**: sus hallazgos se imprimen para revisión
   y no cambian el resultado de la puerta; si no está instalado, el script imprime
   `lintian: not installed, skipped` y sigue con código 0.

El script no comprueba `yt-dlp`, `ffmpeg`, la existencia del `.deb` en `dist/` ni hashes: eso está en
`Depends` y en el checklist manual de release. Termina siempre con una línea `summary:` que resume
paquete, versión, `paths=N/22` y estado de `lintian`.

### 2.3. Flatpak, Snap y AppImage (fuera de alcance)

No se entregan. El motivo es Native Messaging: el host debe existir fuera del sandbox y el manifiesto
del navegador debe apuntar a una ruta autorizada. Si se retoma, hace falta un diseño específico de
portal, `--filesystem` y un host instalado en el sistema; no alcanza con meter `segmenta-host` dentro
del Flatpak. La GUI y el daemon pueden empaquetarse por separado en el futuro, pero el contrato IPC no
cambia.

### 2.4. Extensión de navegador: dos ZIP y carga manual (0.2, en preparación)

- El `.deb` instala `usr/share/segmenta/extensions/segmenta-chromium.zip` y
  `segmenta-firefox.zip`; los dos se construyen desde el mismo árbol `extension/` con
  `node extension/tools/pack.mjs` y `--check` verifica que el contenido empaquetado coincide con las
  fuentes.
- **No hay tienda en 0.2**: ni Chrome Web Store ni AMO, ni XPI firmado. La extensión se carga
  manualmente y la GUI sólo prepara/abre la página de carga (`extension.status`, `extension.prepare`,
  `extension.open`); flujo completo en 09 §7.
- El `.deb` instala los manifiestos nativos de sistema, pero **no** instala la extensión en el perfil
  del navegador.
- El ID de Chromium (`kdeldnifdbpemnhfpmhonlnibbhnoaan`) y el `gecko.id`
  (`segmenta@wilkin.github.io`) están congelados en `extension/CHROMIUM-ID.txt` y
  `crates/segmenta-daemon/src/browsers.rs`; los manifiestos nativos deben coincidir con ellos (09 §2).
- `node extension/tools/sync-shared.mjs --check` garantiza que las copias de `shared/` en chromium y
  firefox no divergen.

---

## 3. Ejecución Segura de Antivirus y Scripts (Cero Shell Injection)

Ejecutar un antivirus o post-procesador sobre archivos descargados es un vector crítico de inyección
si el nombre del archivo contiene caracteres como `; rm -rf /` o `$(malicious)`.

### Regla de Implementación Obligatoria

* **PROHIBIDO:** ejecutar mediante `sh -c "clamscan $FILE"`.
* **OBLIGATORIO:** pasar la ruta como argumento posicional puro (`execve` / `std::process::Command`).
  En 0.2 el hook ya es argv (`postprocess_command: Vec<String>`), así que no se tokeniza ninguna línea
  de shell ni se necesita `shlex`; el siguiente esquema ilustra la invariante con una plantilla de
  texto, que es el caso que sí requiere tokenizar:

```rust
use std::process::Command;
use std::path::Path;

pub fn scan_file_safely(command_template: &str, final_path: &Path) -> std::io::Result<bool> {
    // Dividir argumentos respetando comillas sin invocar una shell
    let parts = shlex::split(command_template)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Comando malformado"))?;

    if parts.is_empty() {
        return Ok(true);
    }

    let program = &parts[0];
    let mut args: Vec<String> = parts[1..].iter().map(|arg| {
        if arg == "%file%" {
            final_path.to_string_lossy().to_string()
        } else {
            arg.clone()
        }
    }).collect();

    // Si %file% no estaba en los argumentos, añadir la ruta al final
    if !parts.iter().any(|p| p == "%file%") {
        args.push(final_path.to_string_lossy().to_string());
    }

    let status = Command::new(program)
        .args(&args)
        .status()?;

    Ok(status.success())
}
```

El hook de 0.2 mantiene `postprocess_command: Vec<String>` (argv, no string de shell), sustituye
`%file%`, `%folder%` y `%name%`, aplica timeout configurable y acota stdout/stderr. Un código de salida
distinto de 0 no convierte una descarga correcta en fallida salvo configuración explícita. Settings,
estados y códigos de error: `12-queue-scheduler-postprocess-auth.md`.

---

## 4. Suite de Pruebas Automatizadas

Recuento vigente en 0.2: **275 tests Rust** (`cargo test --workspace`) y **158 tests Node**
(`node extension/tests/run.mjs`). Q1 debe reconfirmar ambos números; si cambian, se actualiza aquí y en
`05-implementation-roadmap-for-agents.md`, no se deja la cifra vieja.

Cualquier agente que trabaje en el motor debe ejecutar y validar estos casos:

### 4.1. Pruebas Unitarias (`cargo test -p segmenta-core`)
1. **`test_content_range_parsing`:** cabeceras `bytes 0-1/1048576`, formatos sin tamaño total
   `bytes 0-1/*` y respuestas malformadas.
2. **`test_token_bucket_rate_limiter`:** límite a 1 MB/s; al transferir 2 MB el tiempo es
   $2.0 \pm 0.1\text{ s}$.
3. **`test_safe_hot_split_invariants`:** segmento $[0, 9999]$ con `next_offset = 3000`, split en
   midpoint $6500$ → donante $[0, 6500]$ y nuevo $[6501, 9999]$, sin solapamientos ni huecos.
4. **`test_pwrite_all_eintr`:** `EINTR` intermitentes y escrituras parciales; el 100% del buffer se
   vuelca al archivo.

### 4.2. Pruebas de Integración con `wiremock`
1. **Flujo nominal multi-conexión:** `206 Partial Content`, 10 MB en 8 hilos, SHA-256 idéntico.
2. **Servidor que ignora rangos:** `200 OK` completo ante `Range`; el motor conmuta a single-stream sin
   crashear.
3. **HTTP 429 y `Retry-After`:** `429` con `Retry-After: 2`; el worker duerme 2 s y reanuda.
4. **Recuperación tras desconexión abrupta:** `drop(socket)` a mitad de transferencia; `download.resume`
   no vuelve a pedir los primeros bytes.
5. **`If-Range` y ETag cambiado:** el motor reinicia desde el byte 0.

### 4.3. Pruebas de 0.2 (nuevas)
- **Scheduler y `max_active`:** ventana abre/cierra con reloj pausado, no se supera `max_active`, una
  tarea terminada libera slot, `queue.schedule` inválido devuelve error, acción al completar una sola
  vez.
- **Post-proceso:** hook argv con `%file%`, archivo con espacios, ausencia de shell, timeout.
- **Antivirus:** fake `clamscan`/`clamdscan` limpio, infectado, ausente y timeout; cuarentena y nunca
  `completed`.
- **Site logins:** Basic correcto contra servidor local, contraseña incorrecta, sin keyring
  (`1031`), secreto nunca en logs ni en `settings.get`, no enviar credenciales en redirect externo.
- **Torrents:** probe multiarchivo (total y `files[]`) y single-file.
- **Extensión:** captura, watchdog y manifiestos (permisos exactos de 09 §6); cold-start con
  `downloads.search` falso y worker reiniciado.
- **GUI:** `xvfb-run -a cargo test -p segmenta-gui`; no se aceptan skips por falta de display.

---

## 5. Benchmarks Medibles y Verificables

1. **Memoria (< 30 MB RSS):** descarga de 10 GB con 16 conexiones saturando 1 Gbps;
   `valgrind --tool=massif target/release/segmenta-daemon`. Criterio: pico RSS < **30 MiB**.
2. **Cero ensamblado:** $\Delta t = t_{\text{fsync\_close}} - t_{\text{last\_pwrite}}$; criterio
   $\Delta t < \mathbf{0.05\text{ s}}$.
3. **Fluidez de GUI (60 FPS):** `GTK_DEBUG=interactive segmenta-gui`, 4 descargas a 4 Hz, cero caídas
   de frames.

### 5.1. Puerta de CI

Cada pull request debe pasar, como mínimo:

- los gates exactos de §6;
- tests de integración con `wiremock` y un servidor local de rangos;
- build del `.deb` en un contenedor Ubuntu 26.04 (o la LTS fijada para desarrollo);
- `packaging/check-package.sh` sobre el `.deb` recién construido: control (`Package`, `Version`,
  `libsecret-tools` en `Depends`, `Description`), las 22 rutas, `desktop-file-validate` y
  `appstreamcli validate --no-net` sobre las copias extraídas; `lintian` opcional e informativo;
- validación de enlaces de documentación y de que `README`/`openspec` no tengan rutas muertas;
- un job separado para la extensión (lint del manifest, `pack.mjs --check` y `sync-shared --check`).

---

## 6. Gates de 0.2 (comandos exactos)

Los de `odd/tasks/segmenta-0.2.md`, sin variantes:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node extension/tests/run.mjs
node extension/tools/sync-shared.mjs --check
node extension/tools/pack.mjs --check
packaging/check-package.sh dist/segmenta_0.2.0-1_amd64.deb 0.2.0
```

Para GUI: `xvfb-run -a cargo test -p segmenta-gui`; no se aceptan skips por falta de display. Un gate
en rojo bloquea el `done` de la unidad y el tag `v0.2.0`.

---

## 7. Empaquetado de medios, navegadores y antivirus (0.2, en preparación)

- `segmenta-host` → `/usr/bin/segmenta-host` (framing Native Messaging, sin shell).
- Manifiestos nativos de sistema instalados por el `.deb` (§2.1): Chrome, Chromium, Edge, Brave,
  Vivaldi, Opera y Firefox. `extension/install-native.sh` cubre los manifiestos de usuario de todos los
  navegadores detectados.
- Dependencias de ejecución declaradas en `Depends`: `libsecret-tools`, `ffmpeg` y `yt-dlp`; `libgtk-4-1`
  y `libadwaita-1-0` las resuelve `$auto`. **ClamAV no se declara**: es opcional y lo aporta el
  sistema.
- Assets de release además del `.deb` y su `.sha256`:
  - `segmenta-extension-0.2.0.zip` (Chromium MV3, carga manual en modo desarrollador),
  - `segmenta-firefox-0.2.0.zip` (carga manual desde `about:debugging`).
- La extensión **no** se instala desde el `.deb`: se carga manualmente desde el ZIP instalado o desde
  una copia del árbol `extension/` (09 §7).

---

## 8. Licencia y artefactos

El workspace declara `GPL-3.0-or-later`; el repositorio incluye `LICENSE` con el texto completo y el
`.deb` instala `usr/share/doc/segmenta/copyright`. Los binarios y el `.deb` se firman; los checksums se
publican con el artefacto. No se commitean claves privadas de firma ni secretos de tienda.
