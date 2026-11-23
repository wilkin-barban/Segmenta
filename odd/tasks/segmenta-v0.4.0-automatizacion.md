# Segmenta v0.4.0 — automatización, extracción y asistentes

Fase 1. Depende del cierre documentado de v0.3.1. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 1, 4, 8 y 12; [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan, propuestas sujetas a validación.

## Specs

- **S1.** “Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.

## Tasks

- [ ] T1 — Validar contratos IPC, capacidades del backend torrent y precedentes de postprocesado e inhibición antes de elegir APIs (S1) — ruta: core, daemon e IPC — en investigación; commit: pendiente.
- [ ] T2 — Planificar e implementar cliente MCP por stdio para listar, agregar y actuar sobre tareas, reutilizando IPC y definiendo instalación sin modificar configuración ajena sin consentimiento (S1) — ruta: nuevo crate MCP y documentación — commit: pendiente.
- [ ] T3 — Integrar extracción opcional tras verificación, formatos y contraseñas, manejo de multipartes y herramienta externa; conservar original salvo opción explícita y éxito comprobado (S1) — ruta: postprocesado daemon — commit: pendiente.
- [ ] T4 — Inhibir suspensión mientras haya transferencias activas y liberar al pausar, terminar o fallar; degradar si D-Bus no está disponible (S1) — ruta: scheduler/daemon — commit: pendiente.
- [ ] T5 — Exponer modo secuencial para torrents y acción de reproducción, condicionado a soporte comprobado del backend y del archivo (S1) — ruta: torrent, IPC y GUI — commit: pendiente.
- [ ] T6 — Documentar configuración, dependencias, limitaciones y ejecutar gates de release (S1) — ruta: documentación y pruebas — commit: pendiente.

## Criterios de aceptación

- MCP comunica por IPC, mantiene stdout válido para protocolo y reporta fallos sin divulgar secretos.
- Extracción solo procede cuando corresponde; protege destino, informa errores y nunca elimina originales tras fallo.
- Inhibición refleja actividad real; secuencial puede activarse/desactivarse y no promete reproducción si el backend no la soporta.
- Funciones y limitaciones verificadas, con documentación y compatibilidad de clientes existentes.

## Pruebas previstas

- Contrato MCP por stdio con daemon simulado: listado, alta, acciones y errores.
- Archivos de prueba: formatos soportados, contraseña, multipartes, rutas maliciosas, herramienta ausente y extracción fallida.
- D-Bus simulado: transiciones, liberación y ausencia de servicio; torrent local: secuencial, pausa y reproducción cuando sea viable.
- Tests previos de IPC, postprocesado y colas, más suite aplicable y pruebas manuales registradas.

## Riesgos

- Archivos hostiles, enlaces y rutas de extracción pueden sobrescribir datos; ejecución externa y eliminación requieren controles.
- Disponibilidad de APIs de torrent, D-Bus y herramientas es una hipótesis; validar antes de comprometer detalles.
- MCP expone acciones destructivas existentes: respetar límites y permisos del IPC.

## Log

- L1 — Pedido del usuario (verbatim):

> Che, necesito que investigues la propuesta técnica en docs/openspec/13-mdm-parity-and-advanced-roadmap.md (y su resumen en docs/roadmap-mdm-parity.md). En base a esa hoja de ruta, armá los artefactos ODD correspondientes en odd/tasks/ planificando la ejecución escalonada por fases para cada release:

> - Fase 0 (odd/tasks/segmenta-v0.3.1-estabilizacion.md): Auditoría, detección e investigación de errores, bugs y deuda técnica de la release actual v0.3.0 para pasar en limpio la base antes de encarar features nuevas.
> - Fase 1 (odd/tasks/segmenta-v0.4.0-automatizacion.md): Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).
> - Fase 2 (odd/tasks/segmenta-v0.5.0-ecosistema.md): Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).
> - Fase 3 (odd/tasks/segmenta-v0.6.0-red-remoto.md): Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).

> Cada documento ODD debe seguir estrictamente el estándar: encabezado, Specs con S#, Tasks con T# vinculadas a sus S#, criterios de aceptación, pruebas previstas y riesgos.
> Ojo: NO implementes nada de código todavía. Tu tarea ahora es exclusivamente de investigación, auditoría y diseño de los artefactos ODD sin sobreingeniería. Dejame los planes armados y reportame el resultado.

- L2 — Inicio autorizado tras publicación v0.3.1 (pedido textual):

> Abrí odd/tasks/segmenta-v0.4.0-automatizacion.md para dar inicio formal a la Fase 1 y comenzá con la tarea T1 (validar contratos IPC, capacidades torrent y postprocesado/inhibición).

- L3 — Fase 1 iniciada tras la [publicación v0.3.1](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.3.1). T1 en investigación; aún no se autorizó implementación de T2–T6.
- L4 — Investigación inicial de T1: el daemon usa `download.add/list/pause/resume/delete` (`crates/segmenta-daemon/src/manager.rs`) y socket `$XDG_RUNTIME_DIR/segmenta/daemon.sock`, no los nombres `task.*` ni la ruta propuesta en SPEC-013. La versión fijada de `librqbit` es 9.1.0 y `torrent.rs` usa opciones por defecto, sin modo secuencial explícito; capacidad de cambiar prioridad en caliente aún no validada. El postprocesado actual usa argv directo y límites de tiempo/salida; no existe extracción automática de descargas. El daemon no declara `zbus` directamente; la elección del bus y la disponibilidad de inhibición deben verificarse antes de implementar. Se identificaron hipótesis de ciclo de vida torrent y directorios, no reproducidas; no se las presenta como defectos confirmados. T1 permanece abierta hasta validar las capacidades externas y los límites aplicables.
