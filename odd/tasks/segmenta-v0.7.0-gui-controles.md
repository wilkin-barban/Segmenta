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

- [ ] T1 — Foco real al hacer clic en la lupa: el toggle y la acción `win.search` deben compartir un único camino que abra la barra y enfoque el `SearchEntry`; sincronizar el estado del botón en ambos sentidos y darle a la barra el key capture de la ventana (S1, S2) — ruta: `segmenta-gui` — commit: pendiente.
- [ ] T2 — Indicador de VPN y kill switch en la ventana: estado de la interfaz configurada (ausente, caída, activa), distinción visible entre “bloqueado” y “sin VPN”, y acceso directo a la preferencia correspondiente (S4) — ruta: `segmenta-gui` y ajustes — commit: pendiente.
- [ ] T3 — Diálogo de emparejamiento remoto: muestra el estado del control remoto, permite activarlo y desactivarlo, presenta la URL de emparejamiento una sola vez y dibuja el QR sin depender de `qrencode` (S4) — ruta: `segmenta-gui` y cliente IPC — commit: pendiente.
- [ ] T4 — Panel de feeds RSS: lista de suscripciones con nombre, filtro, destino, intervalo y último sondeo; alta, baja y “comprobar ahora”; estado visible cuando una comprobación falla (S4) — ruta: `segmenta-gui` y cliente IPC — commit: pendiente.
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
