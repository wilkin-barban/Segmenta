# Segmenta v0.7.0 — controles visuales de la GUI

Frente abierto después de v0.6.0. Cubre la remediación del buscador (foco al hacer clic) y la
exposición en la ventana de las funciones de las fases 2 y 3: VPN con kill switch, control
remoto y RSS. Estado: plan; sin cambios de código de aplicación todavía.

## Specs

- **S1.** “search_toggle conmuta search-mode-enabled en self.search_bar, pero al hacer clic con el ratón sobre la lupa nunca se transfiere el foco al GtkSearchEntry (a diferencia del atajo Ctrl+F o win.search que sí llaman a focus_search()). El usuario hace clic, empieza a escribir y no busca nada porque el foco queda en la lista.”
- **S2.** “Además, el toggle debe sincronizarse con win.search y el SearchBar debe tener el key capture en la ventana.”
- **S3.** “Remediación del botón de búsqueda con test de UI que simule el clic y verifique el foco.”
- **S4.** “Controles visuales en la GUI para las features de Fase 2 y 3 (estado/indicador de VPN y kill-switch, diálogo para emparejamiento remoto QR, y panel de suscripción/feeds RSS).”
- **S5.** “Redactá el ODD en odd/tasks/ antes de tocar código de aplicación.”

## Tasks

- [x] T1 — Foco real al hacer clic en la lupa: el toggle y la acción `win.search` deben compartir un único camino que abra la barra y enfoque el `SearchEntry`; sincronizar el estado del botón en ambos sentidos y darle a la barra el key capture de la ventana (S1, S2) — ruta: `segmenta-gui` — commit: `89529d5`.
- [x] T2 — Indicador de VPN y kill switch en la ventana: estado de la interfaz configurada (ausente, caída, activa), distinción visible entre “bloqueado” y “sin VPN”, y acceso directo a la preferencia correspondiente (S4) — ruta: `segmenta-gui` y ajustes — commits: `813e1e6` (IPC `vpn.status`), `8880b97` (píldora en el encabezado).
- [x] T3 — Diálogo de emparejamiento remoto: muestra el estado del control remoto, permite activarlo y desactivarlo, presenta la URL de emparejamiento una sola vez y dibuja el QR sin depender de `qrencode` (S4) — ruta: `segmenta-gui` y cliente IPC — commits: `375f7f5` (crate `qrcode` y pintado), `43f7371` (diálogo).
- [x] T4 — Panel de feeds RSS: lista de suscripciones con nombre, filtro, destino, intervalo y último sondeo; alta, baja y “comprobar ahora”; estado visible cuando una comprobación falla (S4) — ruta: `segmenta-gui` y cliente IPC — commits: `3f5bf63` (panel), `6360966` (conexión e IPC), más el ajuste del daemon en `4e1e5b7`.

- [ ] T5 — Documentar los controles nuevos, actualizar capturas y ejecutar los gates de la ventana antes de cerrar el frente (S4) — ruta: documentación y pruebas — commit: pendiente.

## Criterios de aceptación

- Un clic con el ratón en la lupa deja el cursor dentro del campo de búsqueda y escribir filtra la lista; Escape cierra la barra y devuelve el foco a la lista.
- El botón de la lupa y `Ctrl+F` no pueden quedar desincronizados: el estado del botón refleja siempre el modo de búsqueda real.
- La ventana indica el estado de la VPN sin ambigüedad y no sugiere protección cuando el kill switch está desactivado.
- El emparejamiento remoto muestra el token una sola vez, advierte que no se repetirá y permite revocarlo desde la misma ventana.
- Los feeds se administran sin tocar la CLI y un fallo de sondeo se ve como tal, no como una lista vacía.

## Pruebas previstas

- Test de UI bajo Xvfb que simula el clic del botón (no la llamada al método) y verifica el foco del `SearchEntry`, más el cierre con Escape.
- Test de sincronía: activar por acción y comprobar el estado del botón; desactivar por botón y comprobar `search_active()`.
- Indicador de VPN con tres estados simulados (interfaz ausente, caída, activa) y con la VPN sin configurar.
- Diálogo remoto: activación devuelve URL y token una vez, segundo emparejamiento invalida el primero, revocación deshabilita la UI.
- Panel RSS con cliente simulado: alta, baja, `rss.check_now`, y estado de error cuando el daemon rechaza el feed.
- Regresiones de la suite de GUI existente y `xvfb-run -a cargo test -p segmenta-gui`.

## Riesgos

- El emparejamiento remoto crea una credencial: mostrarla en una ventana añade una vía de captura de pantalla, así que el token debe ocultarse tras confirmar y no quedar en el historial de la UI.
- El QR sin `qrencode` obliga a dibujarlo en el proceso o a incrustar una imagen generada; un codificador propio es código nuevo y necesita sus propias pruebas.
- Los estados de VPN dependen de leer la interfaz del sistema desde la GUI: la fuente de verdad debe ser el daemon (`settings`/`system.status`), no una lectura local distinta que pueda contradecirlo.
- La ventana crece en superficie; conviene mantener cada control en su propia página o diálogo para no romper la navegación existente.

## Log

- L1 — Pedido del usuario (verbatim):

> Che, la Fase 3 con v0.6.0 ya quedó cerrada, probada en el host y publicada en el VPS. Ahora vamos con la integración visual en la GUI y la remediación del botón de búsqueda.
>
> Estuve auditando crates/segmenta-gui/src/window.rs y descubrí la causa raíz del botón de búsqueda:
> search_toggle conmuta search-mode-enabled en self.search_bar, pero al hacer clic con el ratón sobre la lupa nunca se transfiere el foco al GtkSearchEntry (a diferencia del atajo Ctrl+F o win.search que sí llaman a focus_search()). El usuario hace clic, empieza a escribir y no busca nada porque el foco queda en la lista. Además, el toggle debe sincronizarse con win.search y el SearchBar debe tener el key capture en la ventana.
>
> Armá el documento ODD para este frente de GUI que cubra:
> 1. Remediación del botón de búsqueda con test de UI que simule el clic y verifique el foco.
> 2. Controles visuales en la GUI para las features de Fase 2 y 3 (estado/indicador de VPN y kill-switch, diálogo para emparejamiento remoto QR, y panel de suscripción/feeds RSS).
>
> Redactá el ODD en odd/tasks/ antes de tocar código de aplicación y avisame cuando lo tengas listo.

- L2 — Estado verificado en el código antes de escribir este plan. El diagnóstico es correcto y **distinto** del que se corrigió en v0.6.0: `search_toggle` (window.rs:1081) se enlaza con `bind_property("active", &self.search_bar, "search-mode-enabled").bidirectional()`, sin pasar por `focus_search()`; en cambio `toggle_search()` (window.rs:564) y la acción `win.search` (window.rs:1521) sí enfocan. Lo que se arregló en v0.6.0 fue la barra invisible por `set_visible(false)` y su lugar bajo el encabezado; la prueba agregada verificaba visibilidad y modo, **no foco**, así que este caso quedó sin cubrir. `set_key_capture_widget` no se usa hoy.
- L3 — Corrección de contexto: la release v0.6.0 se publicó en GitHub (tag y assets), no en un VPS. La sincronización con el VPS de Portafolio fue la de v0.4.0. No hay despliegue de Segmenta en un VPS.
- L4 — Alcance de este documento: solo planificación. No se tocó código de aplicación; T1 a T5 quedan pendientes de autorización de implementación.

- L5 — T1 implementado y probado.
  - El toggle de la lupa se guarda en `Inner` y un `connect_toggled` mueve el caret al `SearchEntry` cuando el modo se activa y lo devuelve a la lista cuando se apaga. Esa es la única vía por la que pasan el clic del ratón y la acción `win.search`, así que ya no pueden divergir; la sincronización del botón sigue siendo la del binding bidireccional, que el test verifica en el sentido contrario (cerrar por acción desactiva el botón).
  - La barra recibe el key capture de la ventana (`set_key_capture_widget`), de modo que Escape y la escritura no dependen de que el foco haya llegado al campo.
  - `grab_focus` se repite a los 600 ms porque la revelación de `GtkSearchBar` es animada y GTK no enfoca un widget sin mapear; el primer intento ocurre igual, para el caso sin animación.
  - Prueba RED primero (los accesos del test no existían: no compilaba) y después verde. La prueba simula el **clic del botón** con `emit_clicked`, no la llamada al método, que era justamente el camino roto. Se comprobó además que la prueba detecta la regresión: al desactivar el manejador de foco el test falla con «the click must move the caret into the search entry».
  - Límite de la prueba, declarado: bajo Xvfb no hay gestor de ventanas, la ventana nunca queda activa y GTK no concede el foco a ningún widget, así que el test observa la **petición** de foco (un contador incrementado por cada `grab_focus` al campo) en lugar de la concesión. Un `search_entry_focused()` que devolviera siempre `false` en ese entorno habría sido una prueba mentirosa, así que se retiró.
  - Gates: `xvfb-run -a cargo test -p segmenta-gui` en verde (72 pruebas entre todos los binarios), `cargo clippy -p segmenta-gui --all-targets -D warnings` y `cargo fmt --check` sin avisos.

- L6 — T2 implementado y probado.
  - Fuente de verdad: el daemon expone `vpn.status` con `{configured, interface, state}` donde `state` es `none`, `active`, `missing`, `down` o `invalid`, calculado con el mismo `netguard::ensure_up` que usa el kill switch. La ventana no lee `/sys` por su cuenta, así que no puede contradecir lo que decide el bloqueo.
  - Ventana: píldora en el encabezado, oculta cuando no hay interfaz configurada (nada sugiere protección), con texto y severidad por estado: `VPN activa (wg0)` en verde, `VPN caída (wg0)` y `VPN ausente (wg0)` en advertencia, `VPN mal configurada (wg0)` en error. Al pulsarla abre las preferencias, que es donde se ajusta la interfaz. Se refresca al arrancar y bajo demanda con `reload_vpn()`.
  - Pruebas: primero RED por compilación (accesos y cliente de pruebas inexistentes) y después verde. La prueba de UI cubre los cuatro estados, que sin configuración no haya píldora, que la severidad sea la esperada para `down`, y que el clic abra las preferencias. Prueba de integración del daemon: `vpn.status` responde `none`, `active` con `lo` y `missing` con una interfaz inexistente.
  - Ajuste de la prueba, declarado: el primer intento comprobaba `active_window()` para ver el diálogo, y bajo Xvfb no hay gestor de ventanas que active nada, así que daba `None` con el código correcto. Se reemplazó por una marca que se enciende al abrir las preferencias; el contenido del diálogo lo cubren sus propias pruebas.
  - Gates: `xvfb-run -a cargo test -p segmenta-gui` en verde (73 pruebas), `cargo test -p segmenta-daemon --test vpn` en verde (5 pruebas), Clippy y fmt limpios.

- L7 — T3 implementado y probado.
  - Dependencia: `qrcode = { version = "0.14", default-features = false }`, pure Rust, sin CMake ni C. La matriz de módulos se convierte a RGBA con zona de silencio y se sube como `gdk::MemoryTexture` a un `gtk::Picture`: no hay archivo temporal ni herramienta del sistema, así que el QR funciona sin `qrencode` instalado.
  - Diálogo (`remote_dialog.rs` + métodos en la ventana): estado del control remoto al abrir, botón «Emparejar» que llama a `remote.enable` y muestra la URL y el token una sola vez, advertencia explícita de que no se pueden volver a leer, y botón «Revocar» que llama a `remote.revoke` y limpia la pantalla. La URL y el token van en `Entry` no editables, para que se puedan seleccionar y copiar con el teclado.
  - Pruebas: RED primero por compilación (métodos y respuestas del cliente simulado inexistentes), después verde. La prueba de UI abre el diálogo, pulsa emparejar, verifica URL con token, token visible, QR pintado (`qr_modules > 0`) y advertencia visible; pulsa revocar, comprueba que el paquete `remote.revoke` salió una vez y que la pareja desapareció de la pantalla; y cierra el diálogo.
  - Gates: `xvfb-run -a cargo test -p segmenta-gui` en verde (74 pruebas), Clippy y fmt limpios.
  - Incidente de entorno, no del código: una corrida posterior mostró 13 fallos y 120 s de duración porque quedó un proceso de `xvfb-run` colgado de una ejecución anterior que había expirado; con el entorno limpio, las mismas pruebas pasan en 9,7 s. Se verificó con `--test-threads=1` (9,78 s) y después en paralelo (9,67 s), sin cambios de código entre ambas.
  - Límite declarado: el diálogo no embebe un botón de copiado propio; los campos son seleccionables y el atajo del sistema alcanza. Si se quiere un botón explícito, es una tarea aparte.

- L8 — T4 implementado y probado.
  - Panel (`rss_panel.rs`): lista de suscripciones con nombre, URL, destino y último sondeo; botones «Comprobar ahora», «Quitar» y «Cerrar»; formulario de alta con nombre, URL, filtro y destino. Los campos de sólo lectura usan etiquetas con elipsis para no romper el ancho de la ventana.
  - IPC: el panel usa los métodos reales del daemon, que son `rss.feed.list`, `rss.feed.add`, `rss.feed.remove` y `rss.check_now` —no `rss.add`/`rss.remove` como decía el pedido—; el panel no descarga feeds por su cuenta, así que las reglas SSRF siguen en un solo lugar.
  - Error visible: el daemon ahora devuelve `failed` y `error` en la respuesta de `rss.check_now` (`4e1e5b7`), y el panel pinta un aviso con la cantidad y el código cuando alguna suscripción no se pudo leer. Antes, un feed roto y una lista vacía se veían igual.
  - Pruebas: RED primero por compilación (panel, accesos y respuestas del cliente simulado inexistentes), después verde. La prueba de UI abre el panel, verifica la fila con nombre, URL, destino y último sondeo; comprueba que una comprobación limpia no muestra error y que una fallida sí; envía el alta con los valores escritos y quita la seleccionada verificando el `feed_id` enviado; y cierra el panel. Prueba de integración del daemon: un feed con URL inexistente responde `failed: 1` y `ERR_RSS_UNREACHABLE`.
  - Gates: `xvfb-run -a cargo test -p segmenta-gui` en verde (75 pruebas), `cargo test -p segmenta-daemon --test rss` en verde (4 pruebas), Clippy y fmt limpios.
  - Límites declarados: el panel no edita una suscripción existente (hay que quitarla y volver a crearla); el estado por feed no se conserva en la base, así que un fallo sólo se ve en la comprobación que lo produjo y no sobrevive al reinicio; y el alta no valida la URL en la GUI, la rechaza el daemon con su código.
