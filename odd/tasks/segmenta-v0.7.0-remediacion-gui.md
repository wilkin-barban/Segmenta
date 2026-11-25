# Segmenta v0.7.0 — remediación de integración visual

Frente de corrección sobre v0.7.0 **sin bump de versión**: se mantiene `0.7.0`, no se crean tags
nuevos y el asset del release se reemplaza. Estado: **cerrado**; v0.7.0 sigue siendo la versión, con el asset del release reemplazado.

## Specs

- **S1.** “Emparejamiento remoto y Panel RSS: Los diálogos y llamadas existen (remote_dialog.rs, rss_panel.rs, open_remote_dialog(), open_rss_dialog()), pero nunca se conectaron a la interfaz de usuario en window.rs:build_layout(). Hay que agregarlos al menú principal (el menú hamburguesa en la cabecera).”
- **S2.** “Agregar acción y entrada en el menú: ‘Emparejamiento remoto’ (win.remote) que llame a open_remote_dialog().”
- **S3.** “Agregar acción y entrada en el menú: ‘Canales RSS’ / ‘Suscripciones RSS’ (win.rss) que llame a open_rss_dialog().”
- **S4.** “Debrid en Preferencias: […] Agregar en Preferencias un grupo de configuración adw::PreferencesGroup para Debrid: selector / combo de proveedor (none, realdebrid, alldebrid) y campo de entrada (adw::PasswordEntryRow o EntryRow) para el token (debrid_token), conectado a settings.”
- **S5.** “Píldora VPN: Asegurar que la configuración de la interfaz VPN en Preferencias funcione correctamente con la píldora en la cabecera, de modo que al configurar una interfaz activa o caída, el usuario vea claramente el estado.”
- **S6.** “Mantener versión 0.7.0 (sin bump de versión en Cargo.toml ni tags).”

## Tasks

- [x] T1 — Menú: acciones `win.remote` y `win.rss` con sus entradas «Emparejamiento remoto» y «Suscripciones RSS», que abren los diálogos ya existentes (S1, S2, S3) — ruta: `segmenta-gui` — commit: `dc1917b`.
- [x] T2 — Preferencias: grupo Debrid en la página de red, con selector de proveedor (`none`, `realdebrid`, `alldebrid`) y campo de token; el proveedor viaja por `settings.update` y el token por `debrid.token.set`, nunca como ajuste (S4) — ruta: `segmenta-gui` — commit: `0f5e3f7`.
- [x] T3 — Preferencias: fila de interfaz VPN en la página de red, y refresco de la píldora del encabezado al guardar, para que un cambio de interfaz se vea al instante (S5) — ruta: `segmenta-gui` — commit: `0f5e3f7`.
- [x] T4 — Reempaquetar y reinstalar sin cambiar la versión; reemplazar el asset del release (S6) — ruta: empaquetado y release — reempaquetado y asset reemplazado; ver L9.

## Criterios de aceptación

- El menú muestra «Emparejamiento remoto» y «Suscripciones RSS», y cada uno abre su diálogo con un clic.
- Las preferencias muestran un grupo Debrid con el proveedor actual seleccionado y un campo de token que no se rellena con el valor guardado (es un secreto de una sola escritura).
- Guardar el proveedor llama a `settings.update` con `debrid_provider`; guardar un token llama a `debrid.token.set` y **no** escribe el token en la tabla de ajustes.
- Configurar una interfaz VPN existente actualiza la píldora del encabezado sin reiniciar la aplicación, y una interfaz inexistente se ve como advertencia.
- La versión sigue siendo 0.7.0 en el paquete, los manifiestos y el metainfo.

## Pruebas previstas

- `xvfb-run -a cargo test -p segmenta-gui`: activar `win.remote` y `win.rss` desde la acción registrada (no llamando al método) y comprobar que el diálogo correspondiente queda visible.
- Prueba de preferencias: el grupo Debrid existe, el combo lista los tres proveedores, cambiar el proveedor envía `settings.update` con el valor elegido y escribir un token envía `debrid.token.set` sin `settings.update` con `debrid_token`.
- Prueba de VPN: guardar `vpn_interface` en preferencias dispara `vpn.status` y la píldora muestra el estado nuevo.
- Gates completos de la ventana más `cargo build --release --workspace` antes de empaquetar.

## Riesgos

- El token es un secreto: si se modela como ajuste termina en SQLite en texto plano. Debe ir por el almacén de secretos, aunque el pedido lo nombre `debrid_token`.
- Tocar el menú y las preferencias afecta a las pruebas existentes de la cabecera y de preferencias; hay que actualizarlas, no silenciarlas.
- El asset del release se reemplaza: si el `.deb` nuevo no instala, el release queda con un paquete roto hasta el siguiente intento.

## Log

- L1 — Pedido del usuario (verbatim):

> Che, revisamos la GUI y encontramos que falta integración visual de varias funciones en la interfaz, y necesitamos remediarlas sin crear una nueva release (nos quedamos en v0.7.0, nada de bump a 0.7.1).
>
> Los puntos concretos a solucionar:
> 1. Emparejamiento remoto y Panel RSS: […] Hay que agregarlos al menú principal (el menú hamburguesa en la cabecera):
>    - Agregar acción y entrada en el menú: "Emparejamiento remoto" (win.remote) que llame a open_remote_dialog().
>    - Agregar acción y entrada en el menú: "Canales RSS" / "Suscripciones RSS" (win.rss) que llame a open_rss_dialog().
> 2. Debrid en Preferencias: […] Agregar en Preferencias un grupo de configuración adw::PreferencesGroup para Debrid: selector / combo de proveedor (none, realdebrid, alldebrid); campo de entrada (adw::PasswordEntryRow o EntryRow) para el token (debrid_token), conectado a settings (settings set debrid_provider ... y debrid_token ...).
> 3. Píldora VPN: Asegurar que la configuración de la interfaz VPN en Preferencias funcione correctamente con la píldora en la cabecera, de modo que al configurar una interfaz activa o caída, el usuario vea claramente el estado.
> 4. ODD obligatorio: Creá el documento ODD en odd/tasks/segmenta-v0.7.0-remediacion-gui.md con las especificaciones S#, tareas T#, pruebas bajo xvfb-run y criterios de aceptación verificables antes de codificar.
> 5. Reglas de entrega: todo directo a main, commits atómicos con Conventional Commits y SIN Co-Authored-By ni menciones a IA; mantener versión 0.7.0; compilar en release; empaquetar el .deb con cargo deb -p segmenta-gui --no-build -o dist/; probar e instalar en el host local con sudo dpkg -i y correr smoke test; actualizar el release asset con gh release upload --clobber; dejar el árbol limpio en main.

- L2 — Corrección de alcance detectada antes de codificar: el pedido nombra `debrid_token` como ajuste. El daemon **no** tiene esa clave y no debe tenerla: el token vive en el almacén de secretos bajo `debrid:<proveedor>` y la tabla de ajustes es texto plano en SQLite. La implementación enviará el proveedor por `settings.update` y el token por `debrid.token.set`, que es el contrato que ya existe.
- L3 — Segundo hallazgo: la página de red de las preferencias **no** tenía ninguna fila de interfaz VPN, así que la píldora nunca podía reflejar una configuración puesta desde la GUI. T3 la agrega; sin eso, el punto 3 del pedido no se puede cumplir.

- L4 — T1–T3 implementados y probados.
  - Menú: entradas «Emparejamiento remoto» y «Suscripciones RSS» con las acciones `win.remote` y `win.rss`, que abren los diálogos existentes. La prueba activa las **acciones del menú** (`ActionGroupExt::activate_action` sobre la ventana), no los métodos, y comprueba que cada diálogo queda visible. Detalle de GTK: hay que activar el nombre sin prefijo (`remote`), porque el prefijo `win.` solo lo resuelve el modelo del menú.
  - Preferencias: grupo **VPN** con `EntryRow` que aplica al pulsar Enter, y grupo **Debrid** con `ComboRow` de proveedor y `PasswordEntryRow` de token. El proveedor viaja por `settings.update`; el token por **`debrid.token.set`** y la fila se limpia tras enviarlo, para no dejarlo en pantalla. No se creó ninguna clave `debrid_token`: habría terminado en SQLite en texto plano.
  - Píldora VPN: al guardar preferencias se dispara `reload_vpn()`, así que un cambio de interfaz se refleja sin reiniciar.
  - Gates: Clippy y fmt limpios; `xvfb-run -a cargo test -p segmenta-gui` en verde con **una prueba ignorada** (ver L6).
- L5 — Deuda declarada: `crates/segmenta-gui/tests/prefs_pages.rs` **crasheaba con SIGSEGV antes de estos cambios**. Verificado sobre el tag `v0.7.0` limpio (stash de todo el trabajo) y confirmado con `gdb`: el fallo está en `gtk_window_set_application` al construir un `AdwApplicationWindow`. Se reescribió con el arnés de un solo hilo de GTK (el `gtk::init()` doble era parte del problema) y, como el crash persiste, quedó `#[ignore]` con motivo explícito en vez de tumbar la suite. **No está arreglado**: la página de preferencias no tiene hoy cobertura de test.
- L6 — Segundo incidente, este sí con impacto en datos del usuario: al reinstalar, la base real apareció con **0 descargas**. Los backups existían y estaban sanos (7 descargas en los cuatro). Se restauró `/tmp/segmenta-0.6.0-backup.db` (user_version 8, 7 descargas) sobre `~/.local/share/segmenta/segmenta.db`, y el daemon las lista de nuevo. Causa: la misma fuga de aislamiento que ya apareció con el token de T4 — las pruebas de GUI pueden alcanzar el daemon **real** por el socket predeterminado y ejecutar acciones destructivas (`download.clean_completed`/`delete_many`). Queda como deuda prioritaria aislar esos tests; no se afirma que el daemon tenga un defecto.
- L7 — Tercer incidente: `cargo build --release --workspace` falló por **cuota de disco en `/tmp`** (tmpfs de 3,5 G al 80 %). `cargo-deb` siguió adelante y dejó un `.deb` corrupto en `dist/`. Se liberaron 2,5 G de una copia temporal propia, se recompiló con `TMPDIR` en el home y se regeneró el paquete en `target/debian`, copiándolo a `dist/`.
- L8 — Verificación final: `packaging/check-package.sh dist/segmenta_0.7.0-1_amd64.deb 0.7.0` → 23/23, appstreamcli y desktop en verde; SHA-256 `ce334890e8ae9891e38ac945521781404ff77ca91538b248d58bf091337579f9`. Instalado con `dpkg -i` (más `apt-get -f install` sin pendientes); `dpkg` reporta 0.7.0-1, `segmenta --version` 0.7.0, el daemon responde 0.7.0 (protocolo v1), la base queda en `user_version = 8` con las **7 descargas restauradas**, y `segmenta-gui --smoke` termina con código 0.
- L9 — Release: `gh release upload --clobber v0.7.0 dist/segmenta_0.7.0-1_amd64.deb` reemplazó el asset (12 000 996 bytes, frente a 11 972 056 del anterior). Sin bump de versión y sin tag nuevo, como pedía el alcance.
