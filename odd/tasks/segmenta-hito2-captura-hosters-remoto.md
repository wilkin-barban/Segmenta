# Hito 2 — Captura, Hosters y Remoto

Estado: planificación cerrada; ejecución por unidades con gates propios.
Contrato: `docs/mdm-parity-architecture.md` (Módulo B, Módulo C, Módulo D y §4).
Versión: **0.7.0**, sin tag ni release hasta cerrar el hito.

## Specs

- **S1 — Extractores nativos.** `segmenta-core/src/sites/` resuelve el enlace real de descarga para
  **GDrive, MediaFire, Gofile, Rapidgator e Internet Archive**, y `hub.rs` hace lo propio con
  **GitHub releases y Hugging Face** con paginación de API, publicando los SHA-256 que el servicio
  ya conoce para que `checksum.rs` los verifique, y aceptando **tokens de Site Logins** desde el
  almacén de secretos. Un enlace no soportado se comporta como hoy: descarga HTTP directa.
- **S2 — FTP y FTPS segmentado.** `segmenta-core/src/ftp.rs` descarga `ftp://` y `ftps://`
  reutilizando `planner::SegmentRange`, `DiskWriter` y `HostLimits`, con sondeo `SIZE` + `MDTM`,
  `REST <offset>` + `RETR` en modo pasivo, y el mismo kill switch de `vpn_interface`.
- **S3 — Captura masiva y ZIP remoto.** `batch.rs` expande patrones secuenciales, `folder.rs`
  rastrea listados `Index of` y directorios FTP con cotas (`MAX_DEPTH`, `MAX_FOLDERS`, `MAX_FILES`),
  `remotezip.rs` lista un ZIP remoto por `Range` (EOCD, ZIP64, directorio central) y descarga
  entradas sueltas con `If-Range` y CRC-32; el diálogo **Link Grabber** de la GUI encola el lote.
- **S4 — Control remoto HTTPS.** `remote.rs` sirve por **HTTPS con certificado autofirmado local**
  (huella SHA-256 visible en `remote_dialog.rs` y `segmenta remote status`), embebe una **PWA móvil**
  y reemplaza el token único por la tabla `remote_devices` con **códigos de emparejamiento
  temporales** (8 dígitos, TTL 10 min, máximo 5 intentos) y revocación individual.
- **S5 — Entrega.** Versión **0.7.0**; todo a `main` en commits atómicos de **≤400 líneas**,
  Conventional Commits, **sin `Co-Authored-By` ni atribución a IA**.
- **S6 — Aislamiento y gates.** Pruebas herméticas: sockets, base y certificados en directorios
  temporales; nunca `~/.local/share/segmenta`, el bus de sesión ni el keyring real. Gates
  completos (`fmt`, `clippy`, workspace y `xvfb-run` GUI) antes de cada push.

## Tareas

- [x] T1 — Andamiaje de `sites/`: tipo `Resolved` (url, headers, nombre sugerido, sha256 opcional), registro por host, y los dos primeros extractores puros con servidor de prueba local (MediaFire e Internet Archive) (S1) — ruta: `segmenta-core` — commit: `8d2db7d` + `ce9993f`.
- [ ] T2 — GDrive (confirmación de análisis de virus y exportación de Docs/Sheets/Slides), Gofile y Rapidgator, con reenvío acotado de formularios y enlaces caducados (S1) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T3 — `hub.rs`: GitHub releases con paginación `Link: rel="next"`, Hugging Face (modelo/dataset) con rutas validadas contra traversal, SHA-256 publicado y token desde Site Logins (S1) — ruta: `segmenta-core` + daemon — commit: pendiente.
- [ ] T4 — `ftp.rs` sin TLS: sondeo `SIZE`/`MDTM`, `REST`/`RETR` pasivo segmentado, reutilizando planner, DiskWriter y HostLimits, con servidor FTP de prueba (S2) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T5 — FTPS, reanudación por `MDTM` y bind a `vpn_interface`, sobre el mismo cliente (S2) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T6 — `batch.rs` (patrones `archivo_[01-25].jpg`) y `folder.rs` (BFS acotado de `Index of` y FTP, filtrado de hosts externos y carpetas padre) (S3) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T7 — `remotezip.rs`: EOCD + ZIP64 + directorio central por `Range`, listado en el alta y descarga selectiva con `If-Range` y CRC-32 (S3) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T8 — Diálogo **Link Grabber** en la GUI y `LinksPart` en `segmenta-host`: pegar listas, filtrar, asignar paquete y contraseña de lote (S3) — ruta: `segmenta-gui`, `segmenta-host` — commit: pendiente.
- [ ] T9 — `remote.rs` en HTTPS con certificado autofirmado local, huella SHA-256 en la UI y renovación al cambiar la interfaz (S4) — ruta: `segmenta-daemon`, `segmenta-gui`, `segmenta-cli` — commit: pendiente.
- [ ] T10 — PWA móvil embebida (estado, alta, filtros, acciones por fila, colas) servida por el mismo puerto (S4) — ruta: `segmenta-daemon` — commit: pendiente.
- [ ] T11 — `remote_devices` (migración aditiva), códigos de emparejamiento con TTL y rate limit, revocación individual y su diálogo (S4) — ruta: `segmenta-daemon`, `segmenta-gui` — commit: pendiente.
- [ ] T12 — Cierre: pruebas en host de los binarios release, notas de 0.8.0 y gates completos (S5, S6) — ruta: documentación — commit: pendiente.

## Criterios de aceptación

- Un enlace de cada servicio soportado (GDrive, MediaFire, Gofile, Rapidgator, Internet Archive, GitHub release, Hugging Face) se resuelve al archivo real; uno no soportado sigue descargándose como hoy, sin error nuevo.
- Los SHA-256 que GitHub o Hugging Face publican llegan a la verificación, y un token de Site Logins ausente produce un error que lo nombra, no un 401 opaco.
- Una descarga FTP/FTPS segmentada reanuda tras corte con `REST`, respeta `HostLimits` y no abre socket fuera de `vpn_interface` cuando está configurada.
- El rastreo de carpetas no sale del host original ni repite rutas padre, y respeta las tres cotas.
- El listado de un ZIP remoto no descarga el archivo completo; una entrada suelta se baja por `Range` y su CRC coincide.
- El control remoto no atiende por HTTP plano: sin TLS no hay servicio, y la huella mostrada coincide con el certificado servido.
- Un dispositivo emparejado se revoca sin afectar a los demás, y un código vencido o agotado no empareja.
- Versión 0.7.0, árbol limpio y gates en verde al cierre de cada unidad.

## Pruebas previstas

- Extractores: servidor HTTP local que imita cada flujo (formulario, token de confirmación, metadatos JSON, redirect a CDN); pruebas puras del parseo sin red.
- Hub: paginación simulada con `Link` y respuestas parciales; SHA-256 comparado contra el payload real; traversal (`../`) rechazado.
- FTP: servidor de prueba en `127.0.0.1` con `PASV`, `SIZE`, `MDTM`, `REST` y corte a mitad de transferencia; reanudación verificada por bytes.
- ZIP remoto: ZIP generado en la prueba, servido con `Range` y `If-Range`; listado sin pedir el cuerpo completo (aserción sobre las peticiones vistas).
- Remoto: certificado generado en directorio temporal, handshake TLS real contra el puerto del daemon de prueba, revocación por dispositivo e intentos de código agotados.
- Todo con `TMPDIR` propio, socket temporal y `secret-tool` de mentira cuando haga falta.

## Riesgos

- **Dependencias nuevas (ítem 5, delivery).** FTPS y HTTPS remoto necesitan TLS de servidor (`tokio-rustls`, `rcgen` o equivalente) y la descompresión de entradas ZIP necesita `flate2`. Se deciden por unidad, con el build base funcionando si la feature no se compila.
- **Contratos de terceros (ítem 3).** Los flujos de GDrive, Gofile y Rapidgator cambian sin aviso: cada extractor debe fallar de forma explícita y dejar la descarga directa como salida, nunca devolver HTML como si fuera el archivo.
- **Seguridad (ítem 2).** Un extractor que acepte un host arbitrario se convierte en SSRF; toda URL resuelta pasa por `urlssrf` y por las guardas de redirect. El certificado autofirmado no debe aceptarse ciegamente en el cliente CLI: se muestra la huella y se fija.
- **Datos (ítem 1).** `remote_devices` y los códigos de emparejamiento son migración aditiva; ningún token en claro, solo digest.
- **Concurrencia (ítem 4).** Un pool de sesiones FTP por descarga y el rastreo BFS concurrente pueden agotar descriptores: cotas duras y cierre garantizado.
- **Alcance.** Doce unidades con dependencias externas: se entregan de a una, con gates y prueba en host, no como un cambio acumulado.

## Log

- L1 — Pedido del usuario (verbatim): «Che, cerramos el Hito 1 de forma impecable en main. Ahora vamos a encarar el Hito 2 (Captura, Hosters y Remoto) siguiendo docs/mdm-parity-architecture.md (Módulos B, C, D y §4). Antes de tocar código de aplicación, armá el plan ODD en odd/tasks/segmenta-hito2-captura-hosters-remoto.md con su investigación previa, specs S#, tareas T#, criterios de aceptación y riesgos. El alcance del hito abarca: 1. Extractores nativos en segmenta-core/src/sites/ (GDrive, MediaFire, Gofile, Rapidgator, Internet Archive) y hub.rs (GitHub releases y Hugging Face con paginación, SHA-256 en checksum y tokens de Site Logins). 2. Cliente FTP/FTPS segmentado en segmenta-core/src/ftp.rs reutilizando SegmentRange, DiskWriter y HostLimits. 3. Link Grabber, Spidering (batch.rs, folder.rs) e inspección/descarga parcial de ZIPs remotos vía Range HTTP (remotezip.rs), con el diálogo en la GUI. 4. Control Remoto HTTPS en el daemon (crates/segmenta-daemon/src/remote.rs) con TLS local autofirmado, PWA móvil embebida y tabla SQLite remote_devices con códigos de emparejamiento. Las mismas reglas de siempre: - Mantenemos la versión en 0.7.0 (sin tag ni release todavía). - Todo directo a main con commits atómicos <=400 líneas, Conventional Commits y sin Co-Authored-By ni atribución a IA. - Tests herméticos con sockets/DB temporales (sin tocar ~/.local/share/segmenta ni keyring real). - Gates completos antes de cada push (fmt, clippy, test workspace y xvfb-run gui). Arrancá investigando el árbol, armá el plan ODD, commitealo y dale luz verde a la primera unidad. Te voy supervisando.»
- L2 — Investigación del árbol: **no existen** `sites/`, `ftp.rs`, `batch.rs`, `folder.rs` ni `remotezip.rs`; el hito es construcción nueva, no refactor. `crates/segmenta-core/src/` tiene hoy `engine.rs`, `probe.rs`, `planner.rs`, `disk.rs`, `hostlimits.rs`, `checksum.rs`, `urlssrf.rs` y `testserver.rs`, que son las piezas a reutilizar. `crates/segmenta-daemon/src/remote.rs` (387 líneas) ya sirve `GET /api/downloads` y `POST /api/action` por **HTTP plano** con un único token en el almacén de secretos y `token_digest`.
- L3 — Investigación de dependencias: el workspace **no** tiene cliente FTP (`suppaftp` ni similar), ni TLS de servidor (`tokio-rustls`/`rcgen`), ni `flate2` en `segmenta-core`. `reqwest` ya trae `rustls-tls` para el cliente, y `zip 0.6` (con `deflate`) sólo está en el daemon. Consecuencia: FTPS y el HTTPS remoto exigen dependencias nuevas, y la descompresión de entradas ZIP en `segmenta-core` también. Cada una se decide en su unidad; T4 (FTP sin TLS) no las necesita y por eso va primero.
- L4 — Límites de alcance de esta entrega: el plan es lo único que se commitea ahora. La primera unidad habilitada es **T1**, que no requiere dependencias nuevas y se prueba entera contra un servidor local.

- L5 — T1 cerrado: extracción nativa con registro por host. `sites::resolve` (y `resolve_with` para pruebas) devuelve `Unsupported` para un host que no reclama nadie, `Resolved` con URL, headers, nombre y SHA-256 opcional, o `Changed { service, reason }` cuando la página ya no tiene la forma esperada. **Nunca devuelve la URL de la página**: guardar un `.html` con nombre de archivo y llamarlo descarga es peor que fallar.
  - Seguridad: todo resultado pasa por `check_result`, que solo acepta `http`/`https`; el llamador sigue aplicando `urlssrf` sobre el destino. Un `href="file:///etc/passwd"` en la página se reporta como `Changed`.
  - MediaFire lee el `href` del `id="downloadButton"`, aceptando el atributo antes o después del id, y solo en rutas `/file/`. Internet Archive resuelve `details/<id>` vía `/metadata/<id>` y elige el archivo real más grande, descartando miniaturas.
  - Pruebas: fetcher inyectable con respuestas canónicas (cinco pruebas de integración) más unitarias del parseo. Nada de red, nada de `testserver`.
  - **Bug corregido en el camino**: `encode_segment` escapaba cada punto (`%2E`), lo que rompe servidores que comparan el nombre literal; ahora conserva `.`, `-`, `_`, `~` y escapa solo lo que cambia la ruta.
  - **Dependencia nueva**: `async-trait` (solo en `segmenta-core`) para el trait `Extractor`. Es la primera del hito y está aislada.
  - Corte de commits: el primer intento quedó en 649 líneas, por encima del tope de 400. Se rehizo en dos unidades (`8d2db7d` registro + MediaFire; `ce9993f` Internet Archive).
