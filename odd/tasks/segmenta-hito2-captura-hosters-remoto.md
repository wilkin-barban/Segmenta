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
- [x] T2 — GDrive (confirmación de análisis de virus y exportación de Docs/Sheets/Slides), Gofile y Rapidgator, con reenvío acotado de formularios y enlaces caducados (S1) — ruta: `segmenta-core` — commit: `d1893fa` + `dbcf591`.
- [x] T3 — `hub.rs`: GitHub releases con paginación `Link: rel="next"`, Hugging Face (modelo/dataset) con rutas validadas contra traversal, SHA-256 publicado y token desde Site Logins (S1) — ruta: `segmenta-core` + daemon — commit: `d7047c7` + `3411ff4`.
- [x] T4 — `ftp.rs` sin TLS: sondeo `SIZE`/`MDTM`, `REST`/`RETR` pasivo segmentado, reutilizando planner, DiskWriter y HostLimits, con servidor FTP de prueba (S2) — ruta: `segmenta-core` — commit: `0e0cba5` + `e17e563` + ver L8.
- [x] T5 — FTPS, reanudación por `MDTM` y bind a `vpn_interface`, sobre el mismo cliente (S2) — ruta: `segmenta-core` — commit: `cca9bd9` + `c4a69f0` + `65c46e8`, ver L9.
- [x] T6 — `batch.rs` (patrones `archivo_[01-25].jpg`) y `folder.rs` (BFS acotado de `Index of` y FTP, filtrado de hosts externos y carpetas padre) (S3) — ruta: `segmenta-core` — commit: `c6b28b7` + ver L10.
- [x] T7 — `remotezip.rs`: EOCD + ZIP64 + directorio central por `Range`, listado en el alta y descarga selectiva con `If-Range` y CRC-32 (S3) — ruta: `segmenta-core` — commit: `a8c2358` + `213b296` + `2360f35`, ver L11.
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

- L6 — T2 cerrado. **GDrive** no lee ninguna página: el id del enlace basta. Un archivo va a `drive.usercontent.google.com/download?...&confirm=t` (el parámetro que responde al intersticial de análisis de virus) y un Docs/Sheets/Slides va a su `export?format=docx|xlsx|pptx`, que es lo que evita bajar un `.gdoc` inservible. **Rapidgator** lee el `location.href` que la página anuncia, con cualquiera de las dos comillas, y reporta `Changed` cuando no hay enlace (caducado, borrado o flujo cambiado).
  - **Gofile** consulta `api.gofile.io/contents/<code>` y elige el archivo más grande del share. El servicio responde `401` sin cuenta: eso se informa como «necesita token de Site Logins», no como página rota. **No se hardcodea ningún token de terceros**; el token se toma de `SEGMENTA_GOFILE_TOKEN` y su cableado a Site Logins es parte de T3.
  - `Fetcher::page_with` agregado para cabeceras por servicio, con implementación por defecto que las ignora: un fetcher de prueba solo implementa lo que necesita.
  - Pruebas: seis nuevas de integración (Drive archivo, Drive Docs ×3, Rapidgator con y sin enlace, Gofile con y sin credenciales) más unitarias del parseo.
  - Corte de commits: 549 líneas juntas se separaron en `d1893fa` (Drive + Rapidgator, 323) y `dbcf591` (Gofile, 226).

- L7 — T3 cerrado: `sites/hub.rs` (GitHub) y `sites/hugging_face.rs`. **GitHub**: `releases/tag/<t>` y `releases/latest` van por la API pública; se elige el asset más grande y se toma `digest: "sha256:…"` cuando el servicio lo publica, que es lo que permite verificar contra la fuente y no solo contra la transferencia. **Paginación** real con `Link: rel="next"`, máximo cinco páginas, con prueba de una primera página vacía que apunta a la segunda.
  - **Hugging Face**: modelos y datasets (`/datasets/…` conserva el prefijo), árbol por API `tree/main?recursive=true`, y el SHA-256 sale de `lfs.oid` — el digest real de los pesos. Las páginas se acumulan antes de elegir, porque el archivo más grande puede estar en la segunda.
  - `Fetcher::fetch` devuelve estado, **cabeceras** y cuerpo; sin eso la paginación era imposible. `page`/`page_with` se mantienen encima, así los extractores anteriores no cambiaron de contrato.
  - Tokens: `SEGMENTA_GITHUB_TOKEN` y `SEGMENTA_HF_TOKEN`. Un repo privado o gated responde `401`/`403` y el mensaje **nombra Site Logins**; el cableado definitivo al almacén de secretos llega con la unidad de la GUI.
  - Pruebas: cuatro de integración nuevas (release con digest, paginación, repo HF con LFS, dataset y repo privado) más unitarias del parseo.
  - Corte: 639 líneas juntas se separaron en `d7047c7` (GitHub, 386) y `3411ff4` (Hugging Face, 253).

- L8 — T4 cerrado (FTP sin TLS). `FtpTarget` lee usuario y contraseña de la URL (`anonymous`/`anonymous@` por defecto); `Control` maneja el formato multilínea de las respuestas; `probe` hace `SIZE` + `MDTM` con una sola conexión de control.
  - **`EPSV` primero, `PASV` de respaldo**, y la IP del `227` **se ignora**: se conecta al host del control. Eso arregla los servidores detrás de NAT y cierra el vector de FTP bounce.
  - Transferencia: `planner::initial_segments` recortado por `HostLimits`, `DiskWriter` con preasignación, una conexión de control + una de datos por segmento con `REST <offset>` + `RETR`, y escritura por offset. `REST` que no responde `350` aborta el segmento: un servidor que dice reanudar y no reanuda corrompería el archivo en silencio.
  - Guardas: `netguard::ensure_up` si hay `vpn_interface`, y comprobación de direcciones privadas contra `urlssrf::is_blocked_ip` **antes** de cualquier socket. `SO_BINDTODEVICE` con `socket2` antes del handshake, como el relay.
  - **Bug encontrado por las pruebas**: con segmentos, el cliente esperaba el `226` final mientras el servidor seguía escribiendo el resto del archivo en un socket que nadie leía. Dos timeouts de 30 s por descarga. Ahora la conexión de datos se cierra en cuanto el rango está completo y no se espera el `226` de un rango cortado a propósito.
  - Pruebas contra un servidor FTP local (`127.0.0.1`, sin filesystem del usuario): sondeo con `SIZE`/`MDTM`, ausencia de `EPSV` con respaldo, credenciales desde el enlace, destino privado rechazado, 6 MB segmentados byte a byte, `REST` con offsets distintos de cero, `HostLimits` bajando de cuatro a una conexión, y corte de stream contabilizado. Nueve pruebas en total.
  - Corte: el commit original de 730 líneas se separó en `0e0cba5` (protocolo) y `e17e563` (servidor de prueba), y este de 267.

- L9 — T5 cerrado (FTPS + reanudación validada). `segmenta-core/src/ftps.rs` es la capa TLS: `TlsMode::{Plain, Explicit, Implicit}` decidido por el esquema (`ftp` / `ftps`), `client_config` con las raíces de la plataforma (`rustls-native-certs`, respaldo `webpki-roots`) y `handshake` genérico sobre cualquier stream, que es lo que permite **actualizar en el lugar** el socket del control.
  - **TLS explícito**: saludo, `USER`/`PASS`, `AUTH TLS` (`234`), handshake sobre el mismo socket y después `PBSZ 0` + `PROT P`. **TLS implícito**: handshake antes del saludo; `PBSZ`/`PROT` se envían igual, porque sin ellos el canal de datos viajaría en claro aunque el control esté cifrado — el error clásico de FTPS.
  - `Control` deja de partir el socket (`split`) y lee las líneas **byte a byte**: un `BufReader` retendría bytes posteriores y el `AUTH TLS` los perdería.
  - **Proveedor criptográfico explícito**: en el grafo hay dos (`ring` de reqwest y otro de librqbit), así que rustls se niega a elegir. Se declara `ring` como dependencia directa con su feature y se instala una vez.
  - **Reanudación**: `FtpConfig::expect_modified` compara el `MDTM` vigente con el que produjo el parcial **antes de emitir cualquier `REST`**; si cambió, la descarga se niega en vez de empalmar dos objetos. Si el servidor no informa `MDTM`, también se niega: sin validador no hay reanudación segura.
  - **Dos bugs encontrados por las pruebas**: (a) el handshake del canal de datos ocurría antes del `RETR`, y el servidor sólo acepta y cifra después de responder `150` → bloqueo mutuo; ahora se conecta, se manda `RETR` y se negocia TLS sobre un par ya establecido. (b) el modo implícito no enviaba `PROT P`, con lo que el servidor no cifraba los datos.
  - Pruebas (15 en `tests/ftp`): FTPS implícito y explícito contra un servidor local con certificado generado **en memoria** (`rcgen`, sólo dev-dependency), `AUTH` después del `PASS`, `MDTM` distinto bloquea antes de `REST` y `MDTM` igual permite la descarga, e interfaz VPN inexistente que rechaza **sin enviar un solo comando** (`ERR_VPN_INTERFACE`).

- L10 — T6 (a) cerrado: `batch.rs`. Patrones `[01-25]` (con el relleno del extremo escrito), `[a-b]` y varios grupos que se multiplican en orden mixto. Tope duro de **1000** URLs: `[1-100000]` se rechaza con el número exacto en el mensaje en vez de materializar cien mil cadenas o truncar en silencio. Un corchete sin cerrar o un grupo que no es rango (`[abc]`) **queda literal**: las URLs llevan corchetes legítimos y adivinarlos rompería enlaces que nadie pidió tocar. Cinco pruebas.
  - `folder.rs`: rastreo BFS de listados `Index of`. Nunca sale del **host original** ni del **directorio de partida**, descarta `../` y `./`, y decodifica `&amp;`/`&quot;`/`&#39;`. Los tres topes (`max_depth`, `max_folders`, `max_files`) están en el resultado: la última entrada dice **por qué** termina la lista, de modo que el usuario no ve un recorte presentado como si fuera el total. Un 404 en una subcarpeta se saltea en vez de abortar el rastreo. Cinco pruebas más dos unitarias del parseo.
  - **Bug corregido durante las pruebas**: la marca de truncamiento se escribía en la última entrada *del momento*, y un archivo posterior la borraba; ahora el motivo se acumula y se aplica al final del recorrido.

- L11 — T7 cerrado: `remotezip.rs`. El archivo se lista leyendo **sólo la cola** (65 557 bytes) para encontrar el *end of central directory* y después el directorio central por `Range`; `total_size` existe porque sin el tamaño no se sabe dónde empieza la cola, y fingirlo fue el primer bug que apareció. **ZIP64** completo: localizador `0x07064b50`, registro `0x06064b50` y el campo extra `0x0001` de cada entrada, con el orden fijo de sus valores.
  - `read_entry` pide el encabezado local (para conocer nombre y extra reales, no supuestos), después sólo el rango de datos de la entrada, infla si es Deflate y **verifica tamaño y CRC-32**; una discrepancia es `Corrupt`, no bytes devueltos. Entrada cifrada y método desconocido se reportan con nombre y código.
  - `HttpRange` es la implementación de producción: `Range` con la misma guarda `urlssrf` que el resto del motor, sin redirecciones automáticas.
  - Pruebas: seis, con archivos ZIP **construidos a mano** (sin dependencia de escritura) para controlar cada byte: listado sin pedir el archivo completo, archivo grande leído desde la cola, cuerpo que no es ZIP, cuerpo demasiado corto, entrada servida por rango y CRC válido, y un byte alterado que debe fallar el CRC.
  - **Decisión de alcance**: no se implementa la *reparación* de un ZIP dañado (eso pertenece a la unidad de extracción y reparación del roadmap, no a esta). Acá un daño se detecta y se informa.
