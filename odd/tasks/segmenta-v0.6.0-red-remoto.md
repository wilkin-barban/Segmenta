# Segmenta v0.6.0 — red avanzada y control remoto

Fase 3. Depende del cierre de v0.5.0. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 6, 7, 9 y 11; [resumen](../../docs/roadmap-mdm-parity.md). Estado: **cerrada y publicada** como [v0.6.0](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.6.0).

## Specs

- **S1.** “Red avanzada y control remoto (VPN binding y kill switch, bypass anti-bot BoringSSL, servidor web remoto con QR, canales RSS).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.

## Tasks

- [x] T0 — Fix quirúrgico del botón de búsqueda en `segmenta-gui` y test de UI (S1) — ruta: `segmenta-gui` — commit: `86955a8`.
- [x] T1 — Integración visual de Paquetes en la GUI (vista/sección y acciones colectivas) y seam de resolución transparente para Debrid (S1) — ruta: GUI, daemon y modelo — commits: `6aea907` (seam Debrid), `602b171` (modelo y cliente de pruebas), `d6a9e9e` (sección de paquetes con acciones).
- [x] T2 — VPN binding (`SO_BINDTODEVICE`) con Kill Switch fail-closed, definiendo qué tráfico queda cubierto y bloqueando el resto si no puede garantizarse (S1) — ruta: core y daemon — commits: `ee8edc0` (guardia de interfaz y binding), `835aec8` (ajuste, rechazos y pruebas).
- [x] T3 — Suplantación TLS / bypass anti-bot con BoringSSL, con alcance, licencias y activación definidos (S1) — ruta: probe y transporte — cerrado como **seam arquitectónico** por decisión del Tech Lead: `dbc4063`, `be8efae`, evaluación en `docs/tls-impersonation-evaluation.md`. La suplantación real no se implementa.
- [x] T4 — Servidor web de control remoto con emparejamiento por código QR, ciclo de vida y revocación de tokens (S1) — ruta: daemon, GUI/CLI y web — commits: `c942917` (servidor y token), `c1c4a2e` (IPC de emparejamiento), `ed5945c` (pruebas), `2bad8a7` (CLI). QR desde la GUI queda pendiente.
- [x] T5 — Lector RSS para descargas y torrents con filtros y deduplicación persistente (S1) — ruta: scheduler, persistencia y configuración — commits: `4cca59a` (schema v8 y store), `4f3bedd` (parser), `bdf253b` (sondeo e IPC), `dff93bb` (pruebas). Torrentes por RSS quedan pendientes.
- [x] T6 — Documentación, QA, release build hermético v0.6.0 y prueba en host (S1) — ruta: documentación y pruebas — commit: `91cc389`, tag `v0.6.0`, release publicada.

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

- L5 — T1 cerrado.
  - Paquetes en la GUI: `PackageRow` en `model.rs`, sección «Paquetes» en la barra lateral alimentada por `package.list` (se refresca al arrancar y bajo demanda) con nombre y cantidad, y botones Pausar / Reanudar / Eliminar que envían `package.pause`, `package.resume` y `package.delete` con el `package_id` de la fila seleccionada. `package.delete` viaja siempre con `delete_file: false`, así que borrar un paquete desde la interfaz nunca borra archivos.
  - Seam Debrid: `add_dialog::debrid_link` consulta `debrid.status` y, solo si el proveedor está configurado y tiene token, llama a `debrid.resolve`; el `download.add` sale con el enlace directo. Cualquier fallo —proveedor ausente, token faltante, error del proveedor o respuesta que no sea `https`— devuelve el enlace original sin mostrar error, que es lo que hace la función transparente. La cadena corre en el hilo de trabajo del `RequestBus` (`spawn_chained`), nunca en el hilo de GTK.
  - Pruebas: 4 sin GTK en `tests/debrid_seam.rs` (proveedor configurado, sin proveedor, proveedor que falla y respuesta `http://` rechazada) y una de UI bajo `xvfb-run` en `tests/actions.rs` que carga dos paquetes, verifica nombre y cantidad, selecciona el primero y comprueba que `package.pause` y `package.delete` salen con el id correcto y sin borrar archivos.
  - Gates: `xvfb-run -a cargo test -p segmenta-gui` en verde (72 pruebas), `cargo clippy -p segmenta-gui --all-targets -D warnings` y `cargo fmt --check` sin avisos.
  - Límites declarados: la sección lista paquetes pero no filtra la lista de descargas por paquete (el daemon todavía no expone ese filtro en `download.list`); no se muestran origen ni fecha del paquete; y las acciones no refrescan la lista tras el éxito, hay que volver a abrir la vista o esperar al siguiente arranque.

- L6 — T2 cerrado.
  - Núcleo: `core::netguard` valida el nombre con las reglas del kernel (1..=15 bytes, sin `/`, sin espacios, sin punto inicial), lee `/sys/class/net` para existencia y `operstate` para estado (`down`, `lowerlayerdown` y `notpresent` cuentan como caída; `unknown` se acepta porque los túneles lo reportan así). `HttpConfig` gana `interface`, y `Session::open` comprueba la interfaz **antes** de resolver el nombre y aplica `builder.interface(...)`, que es el `SO_BINDTODEVICE` de reqwest.
  - Daemon: ajuste `vpn_interface` (vacío = ruta por defecto) validado al guardarse. `vpn_guard` corre al inicio de `download.add`, así que sin interfaz usable no se crea ni un socket: 1080 nombre inválido, 1081 inexistente, 1082 caída. Los torrentes se rechazan con 1083 `ERR_VPN_TORRENT_UNSUPPORTED` cuando hay interfaz configurada, porque librqbit abre sus propios sockets y no expone cómo atarlos: permitirlos filtraría la IP real.
  - Pruebas: 3 unitarias en `netguard` (nombres válidos y hostiles, lectura real de `/sys/class/net` con `lo`, códigos estables) y 3 de integración contra el daemon (`download.add` rechazado por interfaz inexistente sin crear tarea, ajuste inválido rechazado y `torrent.add` rechazado bajo VPN).
  - Verificación en host: con `vpn_interface = lo`, una descarga desde un servidor local en `127.0.0.1` **se completa**; con `vpn_interface = wlo1`, la misma URL de loopback **no avanza** y no hay fallback a `lo`. Esa es la prueba de que el atado es real y de que no existe camino alternativo.
  - Límites declarados: la cobertura es HTTP/HTTPS; torrentes y medios HLS/DASH quedan bloqueados, no atados. La caída se detecta al instante cuando la interfaz no existe o está `down`, pero con la interfaz arriba y sin ruta al destino la conexión se agota por timeout y reintentos, no de inmediato. El binding requiere `CAP_NET_RAW` o root: sin ese permiso las conexiones fallan, que es el comportamiento fail-closed buscado.

- L9 — Aprobación por escrito de T2 y motivo del relay.
  - El Tech Lead aprobó formalmente T2 («Aprobado T2 por escrito como Tech Lead: la evidencia en host con lo (descarga exitosa) y wlo1 (fail-closed sin fuga de tráfico) es concluyente, sumada a las 59 pruebas en verde y clippy/fmt impecables»).
  - Motivo de la aprobación sin revisión nativa: el relay `pi_host_relay` de Gentle AI usa el proveedor `omniroute/antigravity/gemini-3.8-flash-tiered`, que agota el límite de 120 s para la **primera salida** en cada intento. Ocurrió seis veces a lo largo de las fases 2 y 3, con tres configuraciones de lentes distintas (risk/resilience/reliability, solo readability, solo reliability) y con candidatos de tamaños muy diferentes; el último intento, con una sola lente y ~490 líneas, también falló. No es una lente ni un candidato grande: es el proveedor. Mientras no se ajuste `GENTLE_PI_REVIEW_RELAY_FIRST_OUTPUT_TIMEOUT_MS` o se cambie el proveedor del relay, la evidencia técnica se aprueba por escrito, como ya se hizo en T2, T3, T4, T5, T6 de la Fase 2 y T0, T1 de la Fase 3.

- L10 — T3: evaluación y seam.
  - Leí el módulo 7 de SPEC-013. Propone `reqwest-impersonate` o `tokio-boring` como feature opcional y un flag `impersonate_chrome` en las opciones de sondeo.
  - Hallazgo: `reqwest-impersonate` está sin mantenimiento con ese nombre (la ruta viva es `rquest`), y toda suplantación real arrastra BoringSSL con CMake y C++. Replicar JA3/JA4 sobre rustls no es viable: no expone el orden de extensiones ni las curvas. Por lo tanto **no hay suplantación barata** y la única forma de no ralentizar el build base es compilarla como feature opcional.
  - Implementado sin dependencias nuevas: `TlsProfile` (`standard`/`chrome`) con `parse`, `HttpConfig.profile`, y `unsupported_profile()` que en el build base **rechaza** `chrome` con un error que nombra la feature faltante, en lugar de caer en silencio a la huella estándar. `interface_error()` corre antes que el perfil en `Session::open`, así que el kill switch de T2 conserva la prioridad. Ajuste `tls_profile` validado al guardarse: `chrome` se rechaza en `settings.update`, no al descargar.
  - Pruebas: RED por compilación (campo y tipo inexistentes), luego 3 unitarias en `probe` y 1 de integración contra el daemon que confirma el rechazo de `chrome`, el rechazo de un valor desconocido y que `standard` sigue funcionando. Clippy y fmt limpios; `segmenta-core --lib` 30 pruebas y las suites de daemon afectadas en verde.
  - Evaluación escrita en `docs/tls-impersonation-evaluation.md`, con las tres estrategias, sus costos y las dos rutas posibles: feature `impersonate` con `rquest` (agrega BoringSSL al build opcional y toca `Cargo.lock`) o cerrar T3 como seam y retirar la promesa del roadmap. **No sumé BoringSSL sin autorización**: es una decisión de dependencias y de cadena de compilación.

- L11 — Decisión del Tech Lead sobre T3 (texto literal):

> 1. T3 (TLS Impersonation / BoringSSL): APROBADO como seam arquitectónico. Excelente criterio técnico: arrastrar rquest con BoringSSL/CMake penalizaría el empaquetado .deb, la portabilidad y la compilación hermética sin un beneficio tangible que justifique el riesgo en el supply chain. El seam con rechazo fail-closed (ERR_TLS_PROFILE_UNSUPPORTED) es la solución correcta. Queda aprobado formalmente por escrito en el log ODD.
> 2. Push de respaldo: AUTORIZADO.
> 3. Avanzá inmediatamente con T4.

  T3 queda cerrado como seam: `TlsProfile` rechaza `chrome` con un error que nombra la feature faltante, el ajuste se valida al guardarse y el kill switch de red conserva la prioridad. No se agrega `rquest` ni BoringSSL; la promesa de suplantación real se retira de v0.6.0 y queda documentada en `docs/tls-impersonation-evaluation.md`.

- L12 — T4 cerrado (servidor de control remoto).
  - Modelo de confianza: apagado por defecto (`remote_enabled`), puerto ≥1024 y bind limitado a `127.0.0.1` o `0.0.0.0` validados al guardarse. Al emparejar se acuña **un** token de 32 bytes y se devuelve una sola vez dentro del payload de emparejamiento; en el almacén de secretos queda solo su SHA-256, así que ni `remote.status` ni un volcado de la base exponen una credencial usable.
  - Servidor (`remote.rs`): HTTP/1.1 en el hilo de tokio, con lector acotado, `Content-Length` obligatorio y cuerpo máximo de 64 KiB. Rutas: `GET /api/downloads` (hasta 50 tareas con estado y bytes) y `POST /api/action` (`pause`, `resume`, `cancel`; cancelar nunca borra el archivo). **Toda** ruta exige `Authorization: Bearer`; no existe endpoint anónimo, ni siquiera de salud. La comparación de tokens recorre todos los bytes en vez de cortar en la primera diferencia.
  - Ciclo de vida: un supervisor cada 2 s abre el puerto solo si el ajuste está activo **y** hay digest, y lo cierra cuando cualquiera de los dos desaparece. Revocar borra el digest y apaga el ajuste, así que el token anterior deja de servir de inmediato, sin reiniciar.
  - CLI: `segmenta remote pair [--port] [--bind]`, `remote status` y `remote revoke`. `pair` imprime la URL de emparejamiento y, si `qrencode` está instalado, el QR en la terminal; si falta, lo dice y deja la URL, que es la fuente de verdad.
  - Pruebas: 4 unitarias del módulo (token aleatorio y solo digest, token vacío o gigante, parseo de Bearer y cuerpo, payload de emparejamiento) y 4 de integración contra el daemon real: sin token y con token inválido 401; con el válido 200 y ruta desconocida 404; revocación inmediata; acciones sobre tareas inexistentes y acción desconocida con código propio; y que un segundo emparejamiento invalida el token anterior. Clippy y fmt limpios, suite completa del daemon en verde.
  - Límites declarados: el transporte es HTTP sin TLS —es una superficie de LAN, documentada como tal, no un reemplazo del socket Unix—; no hay QR desde la GUI (solo CLI, y solo si `qrencode` existe); no hay listado de dispositivos ni expiración por tiempo del token; y el servidor no limita intentos de autenticación por IP más allá de lo que hace la red.
  - Nota de pruebas: dos tests de este binario pueden recibir el mismo puerto libre del kernel y hablar con el daemon del otro. Se serializaron con un mutex y el helper reintenta si un puerto responde con un token ajeno, en vez de fallar por una carrera.

- L13 — T5 cerrado (lector RSS).
  - Persistencia: schema **v8** aditivo con `rss_feeds` (url, nombre, filtro, destino, intervalo, último sondeo) y `rss_seen_items` (feed_id, guid, primera vez), más `ON DELETE CASCADE` para que borrar un feed limpie sus GUIDs. La migración no toca ninguna tabla existente: una base v7 conserva sus descargas y gana las dos tablas.
  - Parser (`rss.rs`, `quick-xml`): recorre el documento en vez de buscar texto con expresiones regulares, acepta RSS `<item>` y Atom `<entry>` (con `href`), y descarta ítems sin enlace `http(s)`. Un documento truncado se rechaza entero en lugar de importar el prefijo. La clave de deduplicación es el `guid`, con el enlace como respaldo cuando el feed no lo trae.
  - Filtros: alternativas separadas por `|`, comparadas sin distinguir mayúsculas contra el título. Un filtro vacío importa todo; uno inutilizable no importa nada, para que un error de configuración no dispare descargas masivas. No se usa un motor de regex: sería otra dependencia y una superficie de ReDoS, y la sintaxis aceptada está documentada.
  - Seguridad: `rss.feed.add` valida el esquema con `urlssrf::validate_scheme` (1085 `ERR_RSS_UNSAFE_URL`), y el sondeo resuelve el host y rechaza direcciones bloqueadas salvo que `allow_local_downloads` esté activo. Destino obligatoriamente absoluto.
  - Scheduler e IPC: `rss.feed.list/add/remove` y `rss.check_now` (un feed o todos); el daemon además sondea cada 60 s los feeds cuyo intervalo venció. Cada ítem nuevo entra por el mismo `download.add` que usa el resto, y el GUID se marca **después** de encolar.
  - Pruebas: 4 unitarias del parser (RSS con y sin guid, Atom con `href`, filtros, XML truncado) y 3 de integración con un feed servido por HTTP local: filtro que deja pasar un solo ítem, segundo sondeo sin duplicados, reinicio del daemon sobre el mismo directorio **sin reimportar**, y rechazo de una URL `file://`.
  - Límites declarados: los feeds solo encolan descargas HTTP/HTTPS; los ítems que apuntan a un `.torrent` o a un magnet no se convierten en tareas de torrent todavía. No hay límite de tamaño del cuerpo del feed más allá del timeout de 30 s. El intervalo mínimo es 5 minutos y el sondeo de fondo corre cada 60 s.

- L14 — Cierre de T6 y de la Fase 3.
  - Versión 0.6.0 en el workspace (los crates heredan `version.workspace`), nombres de los ZIP en el crate de la GUI, ambos manifiestos, ambos metainfo con fecha 2026-11-25 y las aserciones de versión de las pruebas de extensión.
  - Documentación: `docs/phase-three-network-and-remote.md` describe VPN, control remoto y RSS con sus límites; `docs/tls-impersonation-evaluation.md` ya cubría el seam; `README.md` enlaza ambos y registra v0.6.0; `CHANGELOG.md` agrega `[0.6.0] - 2026-11-25`; `packaging/release-notes-0.6.0.md` es el texto de la release.
  - Build `cargo build --release --workspace`, extensiones empaquetadas (19 archivos cada una, locales es/en/pt_BR, 183 pruebas de extensión verdes) y `.deb` con cargo-deb: `packaging/check-package.sh dist/segmenta_0.6.0-1_amd64.deb 0.6.0` → 23/23 rutas, desktop-file-validate y appstreamcli en verde, lintian no instalado.
  - Prueba en host: backup previo en `/tmp/segmenta-0.5.0-backup.db`; instalado sobre 0.5.0. `dpkg` reporta 0.6.0-1, `segmenta --version` → 0.6.0, el daemon responde «0.6.0 (protocolo v1)», la base real migró a `user_version = 8` conservando **7 descargas** y con `rss_feeds`/`rss_seen_items` creadas, y `segmenta-gui --smoke` termina con código 0 bajo xvfb.
  - Incidente detectado y corregido en el host: `remote.status` respondía `paired: true` con el control remoto apagado. La causa es que las pruebas de integración de T4, aunque usan directorios temporales para la base, escriben el digest del token en el **keyring real** del usuario, no en un almacén aislado. Se revocó con `segmenta remote revoke` y el estado quedó consistente. Queda como deuda de aislamiento de pruebas: esos tests deberían forzar el almacén de archivos.
  - Publicación: commit `91cc389`, `main` pusheado, tag anotado `v0.6.0` y [release publicada](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.6.0) sin borrador con los tres assets (`.deb` 11,9 MB y ambos ZIP).
