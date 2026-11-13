# Segmenta 0.2.0 — QA real en Ubuntu

Registro del QA ejecutado por el usuario sobre el paquete final **`segmenta_0.2.0-1_amd64.deb`**,
sha256 `d633e3e06e0604d60e9162fb1a8aca02d95308c6d65fbcedf55665ff80742eab`, instalado en Ubuntu.
Sólo se anota lo que se observó; lo que no se pudo probar queda como límite explícito, no como
aprobado.

## Artefactos bajo prueba

| Archivo | Tamaño | SHA-256 |
| --- | --- | --- |
| `segmenta_0.2.0-1_amd64.deb` | 7.240.668 B | `d633e3e06e0604d60e9162fb1a8aca02d95308c6d65fbcedf55665ff80742eab` |
| `segmenta-extension-0.2.0.zip` | 57.159 B | `f11a128fa1d31740c65c81dd176251c2631d85ffab309e851e82d058d7e2452e` |
| `segmenta-firefox-0.2.0.zip` | 56.871 B | `7ce34d9a3173dc62ade4281962d501b24df171fa18e4a7decf6c4beaab0e835c` |

## Verificado en esta sesión

- **Panel de calidades**: 4K / 1080p / 720p / 360p / Solo audio, con 720p preseleccionado.
- **Captura de archivos**: confirmación con nombre, host y carpeta; aceptar y rechazar se comportan
  como el contrato, y la descarga del navegador se conserva cuando no se acepta.
- **Antivirus (EICAR)**: la tarea termina `failed` con **1046**, `error_message` con **un solo**
  prefijo `ERR_ANTIVIRUS_FOUND:` y `quarantine_path` visible. El archivo no queda en Descargas.
- **Site login Basic**: contra servidor local, **401** sin credenciales y **200** con ellas.
- **Scheduler**: una tarea `pending` pasa a `completed` dentro de la ventana y con `max_active`.
- **Torrent multiarchivo**: probe correcto, **30 B / 2 archivos**.

## No verificado, y por qué

- **Cold-start de captura**: no se pudo probar. El perfil de Chrome for Testing que se usaba en el
  equipo (`/tmp/chrome-testing`) ya no existe, así que no había un navegador real con el que
  arrancar el worker y observar la primera descarga. La reconciliación (`downloads.search` al
  arrancar, al instalarse y en un tick del watchdog) está cubierta por tests con un trabajador
  reiniciado y `downloads.search` falso, pero **eso no es evidencia de navegador real**. Queda
  pendiente para una sesión con Chrome for Testing o Firefox instalado.
- **Firefox oficial**: la extensión de Firefox no se ejercitó en navegador en esta pasada.

Estos dos puntos no son fallos del producto ni promesas: son pruebas que no se ejecutaron y quedan
declaradas así.

## Gates previos al paquete

`cargo fmt --check` limpio · `cargo clippy --workspace --all-targets -- -D warnings` sin hallazgos ·
`cargo test --workspace` sin suites fallidas · `node extension/tests/run.mjs` 167 tests ·
`sync --check` y `pack --check` (19+19 archivos) verdes · `packaging/check-package.sh` 22/22 rutas,
`desktop-file-validate` y `appstreamcli` ok (`lintian` no instalado en el equipo).

## Hallazgos que este QA destapó y ya están corregidos

1. **0.1.2**: la captura borraba la copia del navegador antes de que el daemon tuviera bytes.
2. **0.2.0**: `run_postprocess` no ejecutaba el escaneo si no había hook configurado, así que un
   archivo infectado terminaba `completed` (corregido en `6475b27`).
3. **0.2.0**: `error_message` repetía el código y `quarantine_path` no se exponía
   (corregido en `bd0c53a`).

Sin tag, sin Release y sin push: la publicación sigue siendo una decisión posterior a este informe.
