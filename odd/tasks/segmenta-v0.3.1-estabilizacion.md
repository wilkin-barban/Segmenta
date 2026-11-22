# Segmenta v0.3.1 — estabilización de v0.3.0

Fase 0. Auditar y estabilizar la base antes de las funciones nuevas. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md) y [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan; sin implementación ni hallazgos nuevos confirmados.

## Specs

- **S1.** “Auditoría, detección e investigación de errores, bugs y deuda técnica de la release actual v0.3.0 para pasar en limpio la base antes de encarar features nuevas.”
- **S2.** “NO implementes nada de código todavía.” Esta restricción aplica a la preparación del plan; la futura ejecución requiere autorización propia.

## Tasks

- [ ] T1 — Establecer la base v0.3.0 y sus gates; contrastar el código, el estado de la release y la evidencia existente, incluida la bandeja en `odd/tasks/segmenta-0.3.0.md` (S1) — ruta: repositorio y QA — commit: pendiente.
- [ ] T2 — Auditar descargas HTTP/segmentación, persistencia, reanudación, colas, programador, torrent y medios; reproducir y clasificar defectos y deuda con evidencia (S1) — ruta: core y daemon — commit: pendiente.
- [ ] T3 — Auditar GUI, bandeja, IPC, CLI, extensiones, localización y empaquetado; distinguir defectos de bloqueos del entorno (S1) — ruta: GUI, host, extensión y packaging — commit: pendiente.
- [ ] T4 — Priorizar hallazgos reproducibles, corregir los aceptados en unidades revisables y registrar los diferidos y su justificación; sin agregar features (S1) — ruta: superficies afectadas — commit: pendiente.
- [ ] T5 — Ejecutar gates, documentar resultados y deuda residual y decidir si la base está lista para v0.4.0 (S1) — ruta: pruebas y documentación de release — commit: pendiente.

## Criterios de aceptación

- Inventario con severidad, reproducción, evidencia, impacto y disposición por hallazgo; no presentar sospechas como bugs confirmados.
- Defectos priorizados corregidos y verificados o explícitamente diferidos con responsable/decisión; regresiones y documentación asociadas.
- Resultado de cada gate y limitaciones del entorno registrados; la siguiente fase no presupone una base limpia sin evidencia.

## Pruebas previstas

- Reproducir cada bug antes del arreglo y fijarlo mediante prueba pública; conservar comportamiento previo relacionado.
- Ejecutar tests de workspace, GUI/extensiones y validación de paquetes según disponibilidad; registrar comandos y resultados reales.
- Pruebas manuales aisladas de bandeja, programación y descargas en entorno soportado; documentar los casos no ejecutados.

## Riesgos

- Persistencia, concurrencia y contratos compartidos: cambios correctivos pueden afectar datos y flujos existentes.
- Una auditoría no demuestra ausencia de defectos; dependencias de GNOME o servicios externos pueden impedir pruebas reales.

## Log

- L1 — Pedido del usuario (verbatim):

> Che, necesito que investigues la propuesta técnica en docs/openspec/13-mdm-parity-and-advanced-roadmap.md (y su resumen en docs/roadmap-mdm-parity.md). En base a esa hoja de ruta, armá los artefactos ODD correspondientes en odd/tasks/ planificando la ejecución escalonada por fases para cada release:

> - Fase 0 (odd/tasks/segmenta-v0.3.1-estabilizacion.md): Auditoría, detección e investigación de errores, bugs y deuda técnica de la release actual v0.3.0 para pasar en limpio la base antes de encarar features nuevas.
> - Fase 1 (odd/tasks/segmenta-v0.4.0-automatizacion.md): Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).
> - Fase 2 (odd/tasks/segmenta-v0.5.0-ecosistema.md): Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).
> - Fase 3 (odd/tasks/segmenta-v0.6.0-red-remoto.md): Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).

> Cada documento ODD debe seguir estrictamente el estándar: encabezado, Specs con S#, Tasks con T# vinculadas a sus S#, criterios de aceptación, pruebas previstas y riesgos.
> Ojo: NO implementes nada de código todavía. Tu tarea ahora es exclusivamente de investigación, auditoría y diseño de los artefactos ODD sin sobreingeniería. Dejame los planes armados y reportame el resultado.
