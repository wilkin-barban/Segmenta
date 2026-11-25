# Segmenta v0.7.0 — remediación de integración visual

Frente de corrección sobre v0.7.0 **sin bump de versión**: se mantiene `0.7.0`, no se crean tags
nuevos y el asset del release se reemplaza. Estado: en curso.

## Specs

- **S1.** “Emparejamiento remoto y Panel RSS: Los diálogos y llamadas existen (remote_dialog.rs, rss_panel.rs, open_remote_dialog(), open_rss_dialog()), pero nunca se conectaron a la interfaz de usuario en window.rs:build_layout(). Hay que agregarlos al menú principal (el menú hamburguesa en la cabecera).”
- **S2.** “Agregar acción y entrada en el menú: ‘Emparejamiento remoto’ (win.remote) que llame a open_remote_dialog().”
- **S3.** “Agregar acción y entrada en el menú: ‘Canales RSS’ / ‘Suscripciones RSS’ (win.rss) que llame a open_rss_dialog().”
- **S4.** “Debrid en Preferencias: […] Agregar en Preferencias un grupo de configuración adw::PreferencesGroup para Debrid: selector / combo de proveedor (none, realdebrid, alldebrid) y campo de entrada (adw::PasswordEntryRow o EntryRow) para el token (debrid_token), conectado a settings.”
- **S5.** “Píldora VPN: Asegurar que la configuración de la interfaz VPN en Preferencias funcione correctamente con la píldora en la cabecera, de modo que al configurar una interfaz activa o caída, el usuario vea claramente el estado.”
- **S6.** “Mantener versión 0.7.0 (sin bump de versión en Cargo.toml ni tags).”

## Tasks

- [ ] T1 — Menú: acciones `win.remote` y `win.rss` con sus entradas «Emparejamiento remoto» y «Suscripciones RSS», que abren los diálogos ya existentes (S1, S2, S3) — ruta: `segmenta-gui` — commit: pendiente.
- [ ] T2 — Preferencias: grupo Debrid en la página de red, con selector de proveedor (`none`, `realdebrid`, `alldebrid`) y campo de token; el proveedor viaja por `settings.update` y el token por `debrid.token.set`, nunca como ajuste (S4) — ruta: `segmenta-gui` — commit: pendiente.
- [ ] T3 — Preferencias: fila de interfaz VPN en la página de red, y refresco de la píldora del encabezado al guardar, para que un cambio de interfaz se vea al instante (S5) — ruta: `segmenta-gui` — commit: pendiente.
- [ ] T4 — Reempaquetar y reinstalar sin cambiar la versión; reemplazar el asset del release (S6) — ruta: empaquetado y release — commit: pendiente.

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
