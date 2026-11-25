# Segmenta v0.6.0 — red avanzada y control remoto

Fase 3. Depende del cierre de v0.5.0. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 6, 7, 9 y 11; [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan; mecanismos propuestos no están validados.

## Specs

- **S1.** “Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.

## Tasks

- [x] T0 — Fix quirúrgico del botón de búsqueda en `segmenta-gui` y test de UI (S1) — ruta: `segmenta-gui` — commit: `86955a8`.
- [ ] T1 — Integración visual de Paquetes en la GUI (vista/sección y acciones colectivas) y seam de resolución transparente para Debrid (S1) — ruta: GUI, daemon y modelo — commit: pendiente.
- [ ] T2 — VPN binding (`SO_BINDTODEVICE`) con Kill Switch fail-closed, definiendo qué tráfico queda cubierto y bloqueando el resto si no puede garantizarse (S1) — ruta: core y daemon — commit: pendiente.
- [ ] T3 — Suplantación TLS / bypass anti-bot con BoringSSL, con alcance, licencias y activación definidos (S1) — ruta: probe y transporte — commit: pendiente.
- [ ] T4 — Servidor web de control remoto con emparejamiento por código QR, ciclo de vida y revocación de tokens (S1) — ruta: daemon, GUI/CLI y web — commit: pendiente.
- [ ] T5 — Lector RSS para descargas y torrents con filtros y deduplicación persistente (S1) — ruta: scheduler, persistencia y configuración — commit: pendiente.
- [ ] T6 — Documentación, QA, release build hermético v0.6.0 y prueba en host (S1) — ruta: documentación y pruebas — commit: pendiente.

## Criterios de aceptación

- Con binding activo no se afirma protección universal sin demostrar cobertura de DNS, redirecciones, torrent y procesos externos; pérdida de interfaz no causa fallback silencioso.
- Modo TLS opcional no rompe transporte normal ni promete resolver todos los desafíos anti-bot.
- Remoto desactivado por defecto, acceso autenticado y emparejamiento revocable; secretos no expuestos en logs.
- RSS respeta filtros y evita duplicados incluso tras reinicio; migraciones preservan datos.

## Pruebas previstas

- Red aislada: caída y recuperación de interfaz, DNS, redirecciones y tráfico de cada componente cubierto; verificar ausencia de fugas.
- Servidor TLS controlado: rutas con y sin emulación, errores y regresiones; no depender de desafíos externos volátiles.
- Remoto: autenticación, tokens inválidos/revocados, autorización, transporte y QR; RSS con feeds simulados, fallos y reinicios.
- Migraciones en copias aisladas y suites aplicables; registrar límites de plataforma y pruebas no ejecutadas.

## Riesgos

- Una fuga de tráfico o exposición remota compromete privacidad y seguridad; requiere revisión independiente y pruebas de integración.
- Binding de sockets, relay y APIs propuestas pueden no existir o no cubrir todos los procesos; no afirmar garantías sin evidencia.
- RSS y acceso remoto introducen entradas de red no confiables; controlar SSRF, credenciales y límites.
- La propuesta menciona DoH y geolocalización en su tabla pero no los asigna a estas releases: quedan fuera hasta decisión explícita.

## Log

- L2 — Autorización del PO (texto literal):

> Che, arrancamos con la Fase 3, el pulido visual de la GUI de Fase 2 y la remediación del botón de búsqueda.
>
> Hice la investigación técnica del botón de búsqueda y encontré la causa raíz exacta en crates/segmenta-gui/src/window.rs:
> 1. En la línea 1011 hay un self.search_bar.set_visible(false);. En GTK4, GtkSearchBar maneja su propia revelación según search-mode-enabled. Si le clavás set_visible(false), el widget queda con visible: false en el layout engine de GTK y el toggle jamás lo dibuja en pantalla aunque search_mode esté activo.
> 2. Además, en la línea 1046 está como add_bottom_bar en vez de add_top_bar(&self.search_bar) debajo del HeaderBar.
> Eliminá ese set_visible(false), ponelo en add_top_bar, asegurate de que el SearchEntry tome el foco al abrir y agregá el test de regresión.
>
> Plan para esta etapa en odd/tasks/segmenta-v0.6.0-red-remoto.md: actualizá el ODD incorporando como primeras tareas: T0 fix quirúrgico del botón de búsqueda con test de UI; T1 integración visual de Paquetes en la GUI y seam de resolución transparente para Debrid; T2 VPN binding con Kill Switch fail-closed; T3 suplantación TLS / bypass anti-bot con BoringSSL; T4 servidor web de control remoto con emparejamiento por QR; T5 lector RSS con filtros y deduplicación; T6 documentación, QA, release build hermético v0.6.0 y prueba en host.
>
> Actualizá el documento ODD, empezá con T0 bajo TDD con commit atómico en main sin Co-Authored-By, y seguimos avanzando.

- L3 — T0 cerrado. Diagnóstico confirmado en el código: `self.search_bar.set_visible(false)` (línea 1011) y `add_bottom_bar` (línea 1046), tal como reportó el PO. Corrección en `86955a8`: se quitó el `set_visible(false)`, la barra pasa a `add_top_bar` justo debajo del encabezado, el `SearchEntry` toma el foco al abrir (tanto desde `toggle_search` como desde la acción `win.search`) y se agregó `search_bar_visible()` para poder observarlo desde la prueba.
- L4 — Evidencia TDD: la primera corrida falló por el acceso inexistente (RED de compilación) y, con la corrección aplicada, la prueba pasa; además se volvió a introducir temporalmente el `set_visible(false)` para comprobar que la prueba lo detecta —falló con «the search bar must not be hidden with set_visible(false)»— y se retiró de nuevo. Suite completa de la GUI bajo `xvfb-run -a cargo test -p segmenta-gui` en verde (67 pruebas) y `cargo clippy -p segmenta-gui --all-targets -D warnings` sin avisos.


- L1 — Pedido del usuario (verbatim):

> Che, necesito que investigues la propuesta técnica en docs/openspec/13-mdm-parity-and-advanced-roadmap.md (y su resumen en docs/roadmap-mdm-parity.md). En base a esa hoja de ruta, armá los artefactos ODD correspondientes en odd/tasks/ planificando la ejecución escalonada por fases para cada release:

> - Fase 0 (odd/tasks/segmenta-v0.3.1-estabilizacion.md): Auditoría, detección e investigación de errores, bugs y deuda técnica de la release actual v0.3.0 para pasar en limpio la base antes de encarar features nuevas.
> - Fase 1 (odd/tasks/segmenta-v0.4.0-automatizacion.md): Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).
> - Fase 2 (odd/tasks/segmenta-v0.5.0-ecosistema.md): Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).
> - Fase 3 (odd/tasks/segmenta-v0.6.0-red-remoto.md): Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).

> Cada documento ODD debe seguir estrictamente el estándar: encabezado, Specs con S#, Tasks con T# vinculadas a sus S#, criterios de aceptación, pruebas previstas y riesgos.
> Ojo: NO implementes nada de código todavía. Tu tarea ahora es exclusivamente de investigación, auditoría y diseño de los artefactos ODD sin sobreingeniería. Dejame los planes armados y reportame el resultado.
