# Segmenta v0.5.0 — ecosistema web, Debrid y paquetes

Fase 2. Depende del cierre de v0.4.0; paquetes preceden a su uso por CNL y galerías. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 2, 3, 5 y 10; [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan; adaptadores y contratos por verificar.

## Specs

- **S1.** “Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.

## Tasks

- [ ] T1 — Verificar protocolos CNL2, proveedores y almacenamiento de secretos; resolver compatibilidad de licencia antes de cualquier adaptación de MDM (S1) — ruta: investigación y diseño — commit: pendiente.
- [ ] T2 — Diseñar y migrar paquetes preservando tareas existentes; exponer agrupación y acciones colectivas (S1) — ruta: store, IPC y GUI — commit: pendiente.
- [ ] T3 — Integrar CNL2 en localhost con validación y consentimiento adecuados, importación en paquetes y límites de entrada (S1) — ruta: daemon, IPC y GUI — commit: pendiente.
- [ ] T4 — Integrar proveedores Debrid propuestos (Real-Debrid, AllDebrid, TorBox); validar disponibilidad y alcance de otros mencionados en el resumen antes de comprometerlos; proteger credenciales (S1) — ruta: core, daemon y preferencias — commit: pendiente.
- [ ] T5 — Agregar consultas y acciones de descargas recientes al host y popup de Chromium/Firefox (S1) — ruta: host y extensión — commit: pendiente.
- [ ] T6 — Integrar detección y ejecución de gallery-dl y agrupar resultados en paquetes; informar ausencia y fallos (S1) — ruta: medios daemon — commit: pendiente.
- [ ] T7 — Documentar migración, configuración y restricciones; ejecutar gates de release (S1) — ruta: documentación y pruebas — commit: pendiente.

## Criterios de aceptación

- Migración conserva tareas previas y paquetes operan coherentemente tras reinicio.
- CNL limita acceso y solicitudes no confiables; no importa ni encola sin el control previsto.
- Debrid mantiene claves fuera de logs y respuestas; errores de proveedor no producen enlaces o tareas engañosos.
- Popup opera en ambos navegadores soportados; galerías se agrupan y fallan de forma visible si falta la herramienta.

## Pruebas previstas

- Migración desde copia aislada de base anterior, reinicio, progreso y acciones de paquete.
- CNL local con entradas válidas, cifradas, inválidas y origen hostil; proveedores simulados, errores y secretos redactados.
- Contratos Native Messaging y pruebas de interfaz en ambos navegadores; gallery-dl simulado y ausencia del binario.
- Regresiones de tareas, medios e IPC y suite aplicable; pruebas externas reales solo con credenciales y servicios autorizados.

## Riesgos

- Migración de datos y listener HTTP amplían superficie de ataque; validar CSRF, límites y SSRF.
- APIs, cobertura de hosts y credenciales externas cambian; no asumir compatibilidad.
- GPL-3.0 de MDM: atribución por sí sola no demuestra compatibilidad; revisar licencia y origen antes de reutilizar código.

## Log

- L1 — Pedido del usuario (verbatim):

> Che, necesito que investigues la propuesta técnica en docs/openspec/13-mdm-parity-and-advanced-roadmap.md (y su resumen en docs/roadmap-mdm-parity.md). En base a esa hoja de ruta, armá los artefactos ODD correspondientes en odd/tasks/ planificando la ejecución escalonada por fases para cada release:

> - Fase 0 (odd/tasks/segmenta-v0.3.1-estabilizacion.md): Auditoría, detección e investigación de errores, bugs y deuda técnica de la release actual v0.3.0 para pasar en limpio la base antes de encarar features nuevas.
> - Fase 1 (odd/tasks/segmenta-v0.4.0-automatizacion.md): Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).
> - Fase 2 (odd/tasks/segmenta-v0.5.0-ecosistema.md): Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).
> - Fase 3 (odd/tasks/segmenta-v0.6.0-red-remoto.md): Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).

> Cada documento ODD debe seguir estrictamente el estándar: encabezado, Specs con S#, Tasks con T# vinculadas a sus S#, criterios de aceptación, pruebas previstas y riesgos.
> Ojo: NO implementes nada de código todavía. Tu tarea ahora es exclusivamente de investigación, auditoría y diseño de los artefactos ODD sin sobreingeniería. Dejame los planes armados y reportame el resultado.
