# Segmenta v0.4.0 — automatización, extracción y asistentes

Fase 1. Depende del cierre documentado de v0.3.1. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 1, 4, 8 y 12; [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan, propuestas sujetas a validación.

## Specs

- **S1.** “Automatización, descompresión y asistentes (módulos MCP, extract, inhibición D-Bus, torrent secuencial).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.
- **S3.** “Arrancá directamente con T2: planificar e implementar el módulo/servidor MCP por stdio que hable con el daemon por IPC para listar tareas, agregar descargas y cambiar estados.”

## Tasks

- [x] T1 — Validar contratos IPC, capacidades del backend torrent y precedentes de postprocesado e inhibición antes de elegir APIs (S1) — ruta: core, daemon e IPC — acuerdos del PO en L7; commit: este cierre.
- [x] T2 — Planificar e implementar servidor MCP por stdio para listar, agregar y actuar sobre tareas, reutilizando IPC y definiendo instalación sin modificar configuración ajena sin consentimiento (S1, S3) — ruta: nuevo crate MCP y documentación — commit: `da0233e` (servidor y pruebas) y `4b906b6` (README).
- [x] T3 — Integrar extracción opcional tras verificación, formatos y contraseñas, manejo de multipartes y herramienta externa; conservar original salvo opción explícita y éxito comprobado (S1) — ruta: postprocesado daemon — commits: `43082e9` (motor), `45594b0` (pruebas), `d29fcb3` (ajustes y wiring).
- [x] T4 — Inhibir suspensión mientras haya transferencias activas y liberar al pausar, terminar o fallar; degradar si D-Bus no está disponible (S1) — ruta: scheduler/daemon — commits: `b6a041e` (inhibidor), `84abf5f` (pruebas), `39fa96a` (actividad), `afba6c9` (tick inicial).
- [x] T5 — Exponer modo secuencial para torrents y acción de reproducción, condicionado a soporte comprobado del backend y del archivo (S1) — ruta: torrent, IPC y GUI — commit: `af41232`.
- [x] T6 — Documentar configuración, dependencias, limitaciones y ejecutar gates de release (S1) — ruta: documentación y pruebas — commit: `bfc8517` (documentación); gates sin GUI en verde.

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
- L4 — Investigación inicial de T1: el daemon usa `download.add/list/pause/resume/delete` (`crates/segmenta-daemon/src/manager.rs`) y socket `$XDG_RUNTIME_DIR/segmenta/daemon.sock`, no los nombres `task.*` ni la ruta propuesta en SPEC-013. La versión fijada de `librqbit` es 9.1.0 y `torrent.rs` usa opciones por defecto, sin modo secuencial explícito; capacidad de cambiar prioridad en caliente aún no validada. El postprocesado actual usa argv directo y límites de tiempo/salida; no existe extracción automática de descargas. El daemon no declara `zbus` directamente; la elección del bus y la disponibilidad de inhibición deben verificarse antes de implementar. Se identificaron hipótesis de ciclo de vida torrent y directorios, no reproducidas; no se las presenta como defectos confirmados.
- L5 — Fuentes externas consultadas: [`AddTorrentOptions` de librqbit 9.1.0](https://docs.rs/librqbit/9.1.0/librqbit/struct.AddTorrentOptions.html) no documenta un campo `sequential`; sí incluye `output_folder`. La [implementación de streaming 9.1.0](https://docs.rs/librqbit/9.1.0/src/librqbit/torrent_state/streaming.rs.html) expone `ManagedTorrent::stream` asíncrono; eso no demuestra por sí solo que exista un interruptor de prioridad en caliente. El [portal Inhibit](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Inhibit.html) ofrece inhibición de suspensión y liberación por `Request.Close`; su viabilidad headless y el bus/protocolo concreto siguen por validar. Una URL exploratoria `TorrentHandle` devolvió 404.
- L6 — La [documentación oficial de logind](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html) describe `Inhibit` para `sleep` y liberación al cerrar el descriptor devuelto; el [idle-inhibit spec](https://apol.pages.freedesktop.org/xdg-specs/idle-inhibit-spec/latest/ch04.html) distingue inactividad de suspensión. En ese momento eran opciones a evaluar, no una selección de diseño ni una prueba en el host.
- L7 — Decisiones y autorización del PO (texto literal):

> Che, revisé a fondo la investigación de T1 que dejaste documentada en el log (L4 a L6):
>
> 1. Contratos IPC: Impecable el hallazgo. Nada de inventar una capa paralela 'task.*'; reusá directamente los métodos reales del daemon ('download.add', 'download.list', 'download.pause', 'download.resume', 'download.delete') sobre el socket '/run/user/1000/segmenta/daemon.sock'.
> 2. Inhibición D-Bus (T4): Acordado. Nos apoyaremos en 'org.freedesktop.login1' con fallback silencioso si logind no responde, sin complicar el daemon.
> 3. Torrent secuencial (T5): Acordado. Usaremos la capacidad de streaming/priorización inicial que provee librqbit, sin meter abstracciones pesadas.
>
> Con estas decisiones de arquitectura resueltas, tenés luz verde para avanzar:
> - En odd/tasks/segmenta-v0.4.0-automatizacion.md, cerrá formalmente T1 con estos acuerdos asentados y marcala como completada (- [x]) con commit limpio en main.
> - Arrancá directamente con T2: planificar e implementar el módulo/servidor MCP por stdio que hable con el daemon por IPC para listar tareas, agregar descargas y cambiar estados.
> - Como siempre: test-first (contrato JSON-RPC / MCP por stdio), commits atómicos en main (≤400 líneas, Conventional Commits, sin Co-Authored-By de IA) y cero sobreingeniería.
>
> Metele que sigo el ciclo de supervisión de 5 minutos.

  T1 se cierra por decisión del PO: el streaming de librqbit no demuestra un interruptor de prioridad en caliente y la inhibición headless no se probó; esas verificaciones prácticas siguen correspondiendo a T5 y T4. No se autorizó implementar T3–T6 en este pedido.
- L8 — T2 implementada en `crates/segmenta-mcp` y cerrada con dos commits: `da0233e` (servidor stdio y pruebas) y `4b906b6` (README). El servidor habla NDJSON por stdin/stdout, resuelve el socket con la misma cascada que el CLI (`SEGMENTA_SOCKET`, `$XDG_RUNTIME_DIR/segmenta/daemon.sock`, `/tmp/segmenta-<uid>/daemon.sock`) y expone cinco herramientas que reenvían a `download.list/add/pause/resume/delete`, sin métodos arbitrarios ni endpoints nuevos. Pruebas: 1 unitaria de límite de frame y 4 de integración (lifecycle, reenvío, errores y notificaciones con daemon simulado). `cargo fmt --check`, `cargo clippy -D warnings` y `cargo test -p segmenta-mcp` pasaron. Smoke con daemon real: listado vacío correcto y borrado de id inexistente devuelve error de herramienta sin filtrar detalles. La revisión independiente detectó un defecto real de `delete_file` obligatorio que se corrigió con prueba previa. Limitaciones residuales declaradas: los timeouts son por I/O y no un plazo total, los frames sobredimensionados terminan el proceso, los parámetros de `initialize` no se validan, y las pruebas no cubren interoperabilidad con clientes MCP reales. T3–T6 siguen sin autorización.

- L9 — Ejecución de T3–T6 tras la autorización del PO.
  - T3: motor de extracción en `crates/segmenta-daemon/src/extract.rs` con listado previo, rechazo de rutas absolutas y `..`, staging interno con veto a enlaces simbólicos, publicación sin sobrescribir y borrado del original solo con éxito y opción explícita. Ajustes `extract_archives` y `extract_remove_source` (ambos falsos). Contraseña por `SEGMENTA_EXTRACT_PASSWORD`, nunca persistida. Pruebas: 7 en `tests/extract.rs`; `tests/postprocess.rs` y `tests/store.rs` siguen verdes.
  - T4: `crates/segmenta-daemon/src/inhibit.rs` mantiene `systemd-inhibit` mientras hay tareas en vuelo, con liberación al pausar o terminar y degradación silenciosa sin logind. Ajuste `inhibit_sleep` (verdadero). El scheduler sondea cada 2 s y conserva el tick por evento; una regresión propia que retrasaba el primer tick se corrigió en `afba6c9`. Verificación real en el host: el bloqueo `sleep` aparece durante una descarga y desaparece al pausarla.
  - T5: ajuste `torrent_sequential` (falso) y `run_torrent` mantiene abierto un `FileStream` del primer archivo, la única vía pública de librqbit 9.1.0 para priorizar piezas. Limitación real: no existe un interruptor secuencial del torrent completo, así que el orden de reproducción no puede prometerse. Sin prueba con torrent real en la suite.
  - T6: `docs/mcp-and-automation.md` documenta MCP, extracción, inhibición, torrents, dependencias del sistema y limitaciones; el README enlaza el documento. Gates ejecutados: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings` y `cargo test --workspace --exclude segmenta-gui`, todo en verde.
