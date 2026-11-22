# Segmenta v0.6.0 — red avanzada y control remoto

Fase 3. Depende del cierre de v0.5.0. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 6, 7, 9 y 11; [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan; mecanismos propuestos no están validados.

## Specs

- **S1.** “Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.

## Tasks

- [ ] T1 — Investigar viabilidad y límites por plataforma para binding, DNS, torrent y procesos externos; definir qué tráfico queda cubierto y bloquear el resto si no puede garantizarse (S1) — ruta: core, daemon y diseño de red — commit: pendiente.
- [ ] T2 — Implementar binding VPN y kill switch fail-closed con monitoreo y recuperación explícita; documentar permisos y cobertura real (S1) — ruta: red y daemon — commit: pendiente.
- [ ] T3 — Evaluar BoringSSL y alternativas opcionales, compatibilidad de licencias y efectos; implementar emulación TLS solo con alcance y activación definidos (S1) — ruta: probe y transporte — commit: pendiente.
- [ ] T4 — Diseñar e implementar control remoto opt-in, autenticación, transporte y emparejamiento QR; definir ciclo de vida y revocación de tokens (S1) — ruta: daemon, GUI/CLI y web — commit: pendiente.
- [ ] T5 — Implementar RSS/Atom, filtros, sondeo y deduplicación persistente para descargas y torrents (S1) — ruta: scheduler, persistencia y configuración — commit: pendiente.
- [ ] T6 — Documentar modelo de amenazas, limitaciones y pruebas; ejecutar gates de release (S1) — ruta: documentación y pruebas — commit: pendiente.

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

- L1 — Pedido del usuario (verbatim):

> Che, necesito que investigues la propuesta técnica en docs/openspec/13-mdm-parity-and-advanced-roadmap.md (y su resumen en docs/roadmap-mdm-parity.md). En base a esa hoja de ruta, armá los artefactos ODD correspondientes en odd/tasks/ planificando la ejecución escalonada por fases para cada release:

> - Fase 0 (odd/tasks/segmenta-v0.3.1-estabilizacion.md): Auditoría, detección e investigación de errores, bugs y deuda técnica de la release actual v0.3.0 para pasar en limpio la base antes de encarar features nuevas.
> - Fase 1 (odd/tasks/segmenta-v0.4.0-automatizacion.md): Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).
> - Fase 2 (odd/tasks/segmenta-v0.5.0-ecosistema.md): Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).
> - Fase 3 (odd/tasks/segmenta-v0.6.0-red-remoto.md): Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).

> Cada documento ODD debe seguir estrictamente el estándar: encabezado, Specs con S#, Tasks con T# vinculadas a sus S#, criterios de aceptación, pruebas previstas y riesgos.
> Ojo: NO implementes nada de código todavía. Tu tarea ahora es exclusivamente de investigación, auditoría y diseño de los artefactos ODD sin sobreingeniería. Dejame los planes armados y reportame el resultado.
