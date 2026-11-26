# Segmenta Hito 0 — aislamiento de pruebas y reparación de la suite GTK

Hito previo a la paridad MDM. Deja el terreno firme: ninguna prueba puede tocar el daemon real, el
llavero ni la base del usuario, y la suite de la ventana vuelve a cubrir Preferencias.

## Specs

- **S1.** “en crates/segmenta-gui/tests/actions.rs (líneas 431, 539 y 545), los tests the_production_window_is_kept_alive_by_the_application y a_second_activation_reuses_the_existing_window llaman directamente a segmenta_gui::app::build_window_for_client(&application), que en crates/segmenta-gui/src/app.rs (líneas 149 y 220) construye UnixClient::from_env() y spawn del tray contra $XDG_RUNTIME_DIR/segmenta/daemon.sock — alcanzando tu daemon real en vivo si no hay SEGMENTA_SOCKET aislado.”
- **S2.** “Auditá TODAS las suites de tests (segmenta-gui, segmenta-daemon, segmenta-cli, segmenta-host, segmenta-mcp) para detectar cualquier otro punto donde un test pueda conectarse al daemon real del usuario, al D-Bus single-instance de io.github.wilkin.Segmenta, al keyring real (secret-tool) o tocar ~/.local/share/segmenta/segmenta.db.”
- **S3.** “Solucioná el diseño limpiamente (por ejemplo, inyección de factoría/cliente en app.rs para el arranque testeable, socket aislado inexistente o efímero en cada test GUI, IDs de aplicación/flags no únicos en tests, y guarda explícita en entorno de pruebas para que nunca se resuelva el socket por defecto del usuario).”
- **S4.** “Demostrá con evidencia que correr la suite completa con el daemon real vivo deja intactas las descargas y secretos reales.”
- **S5.** “Reparar el SIGSEGV en crates/segmenta-gui/tests/prefs_pages.rs […] Investigá la causa exacta del segfault (con gdb bajo xvfb-run si hace falta), corregilo de raíz sin parches frágiles, quitá el #[ignore] y asegurá cobertura real pasando en verde para todas las páginas de Preferencias (incluidos los grupos VPN y Debrid de v0.7.0).”
- **S6.** “Mantenemos versión 0.7.0 […] verificá todos los gates: cargo fmt --all -- --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace y xvfb-run -a cargo test -p segmenta-gui completo sin #[ignore] pendientes en prefs_pages.”

## Tasks

- [ ] T1 — Auditoría de aislamiento en las cinco suites: inventario de cada punto que resuelve el socket por defecto, abre el llavero, usa el bus de sesión para la instancia única o escribe en `~/.local/share/segmenta` (S1, S2) — ruta: tests de `segmenta-gui`, `segmenta-daemon`, `segmenta-cli`, `segmenta-host`, `segmenta-mcp` — commit: pendiente.
- [ ] T2 — `app.rs` con factoría inyectable: el arranque de producción resuelve el cliente y el tray como hoy, pero el arranque de prueba recibe el cliente y un `bool` de tray, de modo que construir la ventana no pueda alcanzar el daemon (S3) — ruta: `segmenta-gui` — commit: pendiente.
- [ ] T3 — Guarda de entorno de pruebas: con `SEGMENTA_TEST_ISOLATED` (o equivalente) definido, cualquier resolución del socket por defecto falla en vez de caer al del usuario, y los tests que arrancan el arranque real definen un socket efímero o un ID de aplicación único (S3) — ruta: `segmenta-gui` y helper compartido de tests — commit: pendiente.
- [ ] T4 — Evidencia de no contaminación: correr la suite completa con el daemon real vivo y comparar antes/después el conteo de descargas y la ausencia de artefactos nuevos (S4) — ruta: verificación — commit: pendiente.
- [ ] T5 — SIGSEGV de `prefs_pages`: causa raíz con `gdb`/`ASAN` sobre `gtk_window_set_application`, corrección de fondo, `#[ignore]` fuera y cobertura de todas las páginas incluyendo VPN y Debrid (S5) — ruta: `segmenta-gui/tests/prefs_pages.rs` — commit: pendiente.
- [ ] T6 — Gates completos y cierre: fmt, clippy de workspace, `cargo test --workspace` y la suite GUI completa sin pruebas ignoradas (S6) — ruta: verificación — commit: pendiente.

## Criterios de aceptación

- Ningún test de ninguna suite abre el socket por defecto del usuario: con el daemon real detenido o vivo, la suite pasa igual y no crea ni borra filas en su base.
- El arranque de producción no cambia de comportamiento; el de prueba no puede resolver un cliente real por accidente.
- `prefs_pages` corre sin `#[ignore]` y verifica que existen todas las páginas y los grupos VPN y Debrid.
- `cargo test --workspace` pasa, incluida la suite GUI bajo Xvfb, con el daemon real vivo y sin tocar sus datos.
- La versión sigue en 0.7.0 y no se regenera el paquete salvo necesidad.

## Pruebas previstas

- Comparación antes/después del `download.list` real y del archivo `~/.local/share/segmenta/segmenta.db` (mtime y conteo) alrededor de `xvfb-run -a cargo test -p segmenta-gui`.
- Test unitario de la guarda: con la variable de aislamiento, resolver el socket por defecto devuelve error explícito.
- Test del arranque de prueba: construir la ventana con un cliente simulado no invoca el cliente de producción (contador de llamadas del fake).
- `prefs_pages` con el arnés GTK único y un ID de aplicación propio; verificación de las páginas y de los grupos VPN/Debrid.

## Riesgos

- El arranque de producción y el de prueba comparten código: si la factoría se introduce mal, la app real podría arrancar sin tray o sin cliente. Cobertura explícita de ambos caminos.
- Tocar tests existentes puede romper expectativas de acciones y de instancia única; hay que actualizarlas, no desactivarlas.
- El SIGSEGV es un fallo del toolkit: una corrección frágil (por ejemplo, evitar `activate()`) puede esconder el problema. Hay que identificar la causa y demostrarla.
- El daemon real del usuario está vivo durante la verificación; cualquier error de aislamiento vuelve a costar datos. Se toma un backup antes de la corrida.

## Log

- L1 — Pedido del usuario (verbatim, resumido en lo esencial): cerrar el Hito 0 antes de avanzar con paridad MDM; commitear `docs/mdm-parity-architecture.md` por separado; aislar herméticamente las pruebas (S1–S4); reparar el SIGSEGV de `prefs_pages` (S5); mantener 0.7.0, commits atómicos sin atribución a IA, gates completos y push al cerrar.
- L2 — Contexto: el incidente que motiva el hito ya ocurrió. En la sesión del 2026-11-25 la base real apareció con 0 descargas y hubo que restaurarla desde `/tmp/segmenta-0.6.0-backup.db`; se documentó en `odd/tasks/segmenta-v0.7.0-remediacion-gui.md` (L6). Este hito es la reparación de esa causa.
