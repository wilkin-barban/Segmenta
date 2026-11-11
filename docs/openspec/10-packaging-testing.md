# OpenSpec 10: Empaquetado, Ejecución Segura y Suite de Pruebas

> **Especificación:** OPENSPEC-SEGMENTA-010  
> **Versión:** 1.0.0  
> **Área:** Configuración de Build Cargo, Empaquetado `.deb` para Ubuntu 26.04, Suite de Pruebas y Benchmarks
>
> **Alcance de entrega v1:** paquete `.deb` y árbol `cargo` reproducible. Flatpak/Snap/AppImage
> quedan fuera de v1; el Native Messaging fuera del sandbox no se resuelve con un manifiesto
> dentro del paquete y requiere una decisión de producto posterior.

---

## 1. Definición del Workspace `Cargo.toml`

El repositorio de código se estructura con Rust (edición 2021/2024) y MSRV (Minimum Supported Rust Version) **1.80.0**:

```toml
[workspace]
resolver = "2"
members = [
    "crates/segmenta-core",
    "crates/segmenta-daemon",
    "crates/segmenta-host",
    "crates/segmenta-cli",
    "crates/segmenta-gui",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
authors = ["Segmenta Contributors"]
license = "GPL-3.0-or-later"

[workspace.dependencies]
tokio = { version = "1.40", features = ["full"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream", "cookies"] }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
gtk4 = { version = "0.9" }
libadwaita = { version = "0.7" }
clap = { version = "4.5", features = ["derive"] }
uuid = { version = "1.10", features = ["v4"] }
nix = { version = "0.29", features = ["uio", "user", "socket"] }
libc = "0.2"
shlex = "1.3"
secret-service = "4.0"
wiremock = "0.6"
tempfile = "3.12"
```

---

## 2. Empaquetado para Ubuntu 26.04 (`.deb`; Flatpak fuera de v1)

### 2.0. Dependencias del sistema

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  libsecret-1-dev libsecret-tools libssl-dev ffmpeg
cargo install cargo-deb
```

`ffmpeg` es una dependencia de ejecución sólo para HLS/DASH (v1 exige FFmpeg >= 7.0, con el
demuxer/muxer correspondiente) y se comprueba con `ffmpeg -version` al usar la Fase 5; el motor
directo no lo necesita. La licencia de FFmpeg debe revisarse antes de distribuir un binario que lo
enlace; el `.deb` puede declararlo como recomendación en lugar de dependencia obligatoria. `libsecret` se usa vía D-Bus/Secret Service; no se
enlaza una biblioteca que guarde secretos en SQLite. La versión de Ubuntu de desarrollo se fija en CI
sobre Ubuntu 26.04 LTS, la LTS objetiva de esta release.

`libsecret-tools` es **dependencia de ejecución declarada en el `.deb`** (`Depends`), no sólo
dependencia de build: el daemon habla con el Secret Service del escritorio a través de
`secret-tool`. `dpkg-deb --info` debe mostrarla y `packaging/check-package.sh` falla si falta (§2.2.1).
El backend de secretos elige el keyring sólo cuando `secret-tool` existe y una consulta de sondeo
termina con código 0 (secreto encontrado) o 1 (no hay secreto); **nunca** se usa
`secret-tool --version`, que en libsecret 0.21 devuelve 2. Si el keyring no está disponible, el
backend se informa `unavailable`, no se escribe ningún secreto en archivo plano y un alta con
cabeceras sensibles falla con `1031` (`ERR_KEYRING_UNAVAILABLE`).

### 2.1. Estructura de Instalación del Paquete Debian (`.deb`)
Construido mediante `cargo-deb`, el paquete instala los siguientes archivos en el sistema:

| Ruta de Instalación | Función |
| :--- | :--- |
| `/usr/bin/segmenta-daemon` | Demonio central del motor de descargas. |
| `/usr/bin/segmenta-gui` | Interfaz gráfica GTK4 + Libadwaita. |
| `/usr/bin/segmenta-cli` | Herramienta CLI de terminal. |
| `/usr/bin/segmenta-host` | Proceso puente de Native Messaging. |
| `/usr/lib/systemd/user/segmenta.service` | Unidad systemd de usuario para el daemon. |
| `/usr/lib/systemd/user/segmenta.socket` | Activación por socket bajo demanda. |
| `/usr/share/applications/io.github.wilkin.Segmenta.desktop` | Acceso directo en el menú de aplicaciones de Ubuntu. |
| `/usr/share/metainfo/io.github.wilkin.Segmenta.metainfo.xml` | Metadatos para Ubuntu App Center y GNOME Software. |
| `/usr/share/icons/hicolor/scalable/apps/io.github.wilkin.Segmenta.svg` | Icono vectorial de la aplicación. |
| `/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json` | Registro de Native Messaging para Chrome/Brave a nivel de sistema. |
| `/usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json` | Registro de Native Messaging para Firefox a nivel de sistema. |

---

### 2.2. Flujo de build e instalación

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
cargo deb -p segmenta-gui --no-build -o dist/
packaging/check-package.sh dist/segmenta_0.0.2-1_amd64.deb
sudo apt install ./dist/segmenta_0.0.2-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
```

El `.deb` no debe ejecutar `systemctl enable` desde `postinst` sin una decisión explícita: la unidad de
usuario se activa por el operador o por la GUI. El desinstalador conserva `$XDG_DATA_HOME/segmenta` y
pregunta antes de borrar descargas; el paquete no borra archivos del usuario.

### 2.2.1. Puerta de empaquetado (`packaging/check-package.sh`)

Es la puerta obligatoria entre `cargo deb` y cualquier tag o publicación. Se ejecuta sobre un `.deb`
ya construido —no lo genera— y valida el paquete real, no las fuentes:

```bash
packaging/check-package.sh dist/segmenta_0.0.2-1_amd64.deb [version-esperada]
```

Qué comprueba, en este orden y saliendo con código distinto de cero en el primer fallo duro:

1. que el archivo exista y sea un `.deb`;
2. que `dpkg-deb --info` declare `Package: segmenta`, la `Version` esperada (por defecto `0.0.2`,
   sobreescribible con el segundo argumento o `SEGMENTA_EXPECTED_VERSION`), `libsecret-tools` en
   `Depends` y un `Description` no vacío;
3. que `dpkg-deb --contents` liste los diez archivos del contrato de instalación: los cuatro
   binarios (`segmenta`, `segmenta-cli`, `segmenta-daemon`, `segmenta-gui`), las dos unidades de
   usuario (`segmenta.service`, `segmenta.socket`), el `.desktop`, el metainfo, el icono y
   `usr/share/doc/segmenta/copyright`; informa **todos** los que falten, no sólo el primero;
4. que `desktop-file-validate` y `appstreamcli validate --no-net` acepten las copias **extraídas
   del propio paquete** con `dpkg-deb -x` en un directorio temporal de `mktemp -d` que se borra con
   `trap` al salir (nunca una ruta fija como `/tmp/...`);
5. `lintian` si está instalado. Es **opcional e informativo**: sus hallazgos se imprimen para
   revisión y no cambian el resultado de la puerta; si no está instalado, el script imprime
   `lintian: not installed, skipped` y sigue con código 0.

`desktop-file-validate` (paquete `desktop-file-utils`) y `appstreamcli` (paquete `appstream`) sí son
requisitos del gate. El script termina siempre con una línea `summary:` que resume paquete, versión,
rutas encontradas y estado de `lintian`.

### 2.3. Flatpak, Snap y AppImage (fuera de v1)

No se entregan en v1. El motivo es Native Messaging: el host debe existir fuera del sandbox y el
manifiesto del navegador debe apuntar a una ruta autorizada. Si se retoma, hace falta un diseño
específico de portal, `--filesystem` y un host instalado en el sistema; no alcanza con meter
`segmenta-host` dentro del Flatpak. La GUI y el daemon pueden empaquetarse por separado en el futuro,
pero el contrato IPC no cambia.

### 2.4. Extensión de navegador

- `extension/` se empaqueta como ZIP para Chrome Web Store y como XPI firmado para Firefox.
- El ID de Chrome y el `gecko.id` se congelan antes de publicar los manifiestos nativos (09).
- El paquete `.deb` instala los manifiestos nativos de sistema, pero no instala la extensión: el
  usuario la instala desde la tienda o en modo desarrollador.
- El build de release verifica que `allowed_origins`/`allowed_extensions` coincidan con los IDs
  publicados y que no quede una clave de desarrollo.

## 3. Ejecución Segura de Antivirus y Scripts (Cero Shell Injection)

La opción de ejecutar un antivirus o post-procesador sobre archivos descargados es un vector crítico de inyección de comandos si el nombre del archivo contiene caracteres como `; rm -rf /` o `$(malicious)`.

### Regla de Implementación Obligatoria:
* **PROHIBIDO:** Ejecutar mediante `sh -c "clamscan $FILE"`.
* **OBLIGATORIO:** Tokenizar la línea de comando configurada en un array de argumentos y pasar la ruta del archivo directamente como argumento posicional puro utilizando la API del kernel (`execve` / `std::process::Command`):

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

---

## 4. Suite de Pruebas Automatizadas

Cualquier agente que trabaje en el motor debe ejecutar y validar los siguientes casos de prueba:

### 4.1. Pruebas Unitarias (`cargo test -p segmenta-core`)
1. **`test_content_range_parsing`:**
   * Validar cabeceras `bytes 0-1/1048576`, formatos sin tamaño total `bytes 0-1/*`, y respuestas malformadas.
2. **`test_token_bucket_rate_limiter`:**
   * Configurar límite a 1 MB/s. Verificar que al transferir 2 MB el tiempo transcurrido es de $2.0 \pm 0.1\text{ s}$.
3. **`test_safe_hot_split_invariants`:**
   * Simular un segmento de $[0, 9999]$ con `next_offset = 3000`.
   * Ejecutar split en midpoint $6500$.
   * Verificar que el donante queda en $[0, 6500]$ y el nuevo en $[6501, 9999]$, sin solapamientos ni huecos.
4. **`test_pwrite_all_eintr`:**
   * Emular fallos `EINTR` intermitentes y escrituras parciales. Verificar que el 100% del buffer se vuelca al archivo.

### 4.2. Pruebas de Integración con `wiremock`
1. **Flujo Nominal Multi-Conexión:**
   * Servidor mock devuelve `206 Partial Content` y sirve 10 MB en 8 hilos. Validar SHA-256 final idéntico al original.
2. **Servidor que Ignora Rangos:**
   * Servidor devuelve `200 OK` con archivo completo a peticiones con `Range`. Verificar que el motor conmuta a single-stream sin crashear.
3. **Manejo de HTTP 429 y `Retry-After`:**
   * Servidor devuelve `429` con `Retry-After: 2`. Validar que el worker duerme exactamente 2 segundos y reanuda con éxito.
4. **Recuperación tras Desconexión Abrupta:**
   * Cortar la conexión a mitad de la transferencia (`drop(socket)`). Reanudar con `download.resume` y verificar que los primeros bytes no se vuelven a solicitar.
5. **Detección de Archivo Modificado (`If-Range`):**
   * Cambiar el `ETag` en el servidor mock durante la pausa. Verificar que el motor reinicia desde el byte 0.

---

## 5. Benchmarks Medibles y Verificables

Para certificar los objetivos de ingeniería del proyecto:

1. **Benchmark de Memoria (< 30 MB RSS):**
   * Medir consumo durante una descarga de 10 GB con 16 conexiones saturando 1 Gbps:
     ```bash
     valgrind --tool=massif target/release/segmenta-daemon
     ```
   * Criterio de éxito: Pico de memoria residente (RSS) inferior a **30 MiB**.
2. **Benchmark de Cero Ensamblado (Tiempo de Finalización):**
   * Medir el tiempo transcurrido entre el último byte recibido por la red y el cierre del descriptor de archivo:
     $$\Delta t = t_{\text{fsync\_close}} - t_{\text{last\_pwrite}}$$
   * Criterio de éxito: $\Delta t < \mathbf{0.05\text{ segundos}}$ (frente a los varios minutos de los gestores tradicionales).
3. **Benchmark de Fluidez de GUI (60 FPS):**
   * Activar monitor de fotogramas de GTK: `GTK_DEBUG=interactive segmenta-gui`.
   * Simular 4 descargas simultáneas actualizando barras de segmentos a 4 Hz.
   * Criterio de éxito: Cero caídas de frames en el renderizado de la ventana principal.

### 5.1. Puerta de CI

Cada pull request debe pasar, como mínimo:

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo test --workspace`;
- tests de integración con `wiremock` y un servidor local de rangos;
- build del `.deb` en un contenedor Ubuntu 26.04 (o la versión LTS fijada para desarrollo);
- `packaging/check-package.sh` sobre el `.deb` recién construido: control (`Package`, `Version`,
  `libsecret-tools` en `Depends`, `Description`), los diez archivos instalados, `desktop-file-validate`
  y `appstreamcli validate --no-net` sobre las copias extraídas; `lintian` es opcional e informativo;
- validación de enlaces de documentación y de que `README`/`openspec` no tengan rutas muertas;
- un job separado para la extensión (lint del manifest, build del ZIP/XPI) cuando exista código.

### 5.2. Licencia y artefactos

El workspace declara `GPL-3.0-or-later`; el repositorio debe incluir `LICENSE` con el texto completo y
un `NOTICE`/`README` que indique la licencia de la extensión. Los binarios y el `.deb` se firman; los
checksums se publican con el artefacto. No se commitean claves privadas de firma ni secretos de tienda.

---

## 7. Empaquetado de medios y navegadores (0.1.0)

- Nuevo crate `segmenta-host` → `/usr/bin/segmenta-host` (framing Native Messaging, sin shell).
- Manifiestos nativos de sistema instalados por el `.deb`:
  - `/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json`
  - `/etc/chromium/native-messaging-hosts/io.github.wilkin.segmenta.json`
  - `/usr/lib/mozilla/native-messaging-hosts/segmenta@wilkin.github.io.json`
  - `/etc/opt/edge/native-messaging-hosts/io.github.wilkin.segmenta.json`
  Chromium, Brave, Vivaldi y Opera reutilizan la ruta de Chromium/Edge cuando su paquete la respeta;
  `extension/install-native.sh` cubre los manifiestos de usuario de todos los navegadores detectados.
- Dependencias de ejecución: `libsecret-tools`, `ffmpeg` y `yt-dlp` (nombres reales de Ubuntu 26.04).
  No se embeben binarios; se documenta que el sistema puede actualizarlos y que YouTube cambia.
- Assets de release además del `.deb` y su `.sha256`:
  - `segmenta-extension-0.1.0.zip` (Chromium MV3, cargar en modo desarrollador),
  - `segmenta-firefox-0.1.0.zip` (XPI para Firefox, carga temporal desde `about:debugging`).
- `packaging/check-package.sh` valida además: `segmenta-host` en `/usr/bin`, los manifiestos nativos,
  `Depends` con `yt-dlp` y `ffmpeg`, y la existencia de ambos ZIP en `dist/`.
- La extensión **no** se instala desde el `.deb`; se carga desde el ZIP o en modo desarrollador.
