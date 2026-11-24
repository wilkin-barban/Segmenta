# Segmenta v0.5.0 — ecosistema web, Debrid y paquetes

Fase 2. Depende del cierre de v0.4.0; paquetes preceden a su uso por CNL y galerías. Fuente: [SPEC-013](../../docs/openspec/13-mdm-parity-and-advanced-roadmap.md), módulos 2, 3, 5 y 10; [resumen](../../docs/roadmap-mdm-parity.md). Estado: plan; adaptadores y contratos por verificar.

## Specs

- **S1.** “Ecosistema Web, Debrid y paquetes (debrid multihoster, Click'n'Load 2 con paquetes de enlaces, popup en WebExtensions, gallery-dl).”
- **S2.** “NO implementes nada de código todavía.” Aplica a la preparación de estos planes.
- **S3.** “Registres la autorización en el ## Log del archivo ODD y arranques con T1: investigación y diseño de los contratos para Click'n'Load 2 (puerto local 9666, descifrado de enlaces CNL2, orígenes seguros), integración de Debrid y el modelo de paquetes en la base SQLite preservando 100% de compatibilidad con las descargas existentes.”

## Tasks

- [x] T1 — Verificar protocolos CNL2, proveedores y almacenamiento de secretos; resolver compatibilidad de licencia antes de cualquier adaptación de MDM (S1, S3) — ruta: investigación y diseño — hallazgos en L3 y schema propuesto abajo; commit: este documento.
- [x] T2 — Diseñar y migrar paquetes preservando tareas existentes; exponer agrupación y acciones colectivas (S1) — ruta: store, IPC y GUI — commits: `4f1d345` (migración v7 y store), `5c062f9` (IPC y acciones colectivas), `57fe002` (pruebas de integración). Queda la superficie de GUI, diferida a T5/T7.
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

- L2 — Autorización del PO (texto literal):

> Che, el Product Owner dio luz verde formal para arrancar la Fase 2 (v0.5.0 — ecosistema web, Debrid y paquetes) en odd/tasks/segmenta-v0.5.0-ecosistema.md.
>
> Necesito que:
> 1. Registres la autorización en el ## Log del archivo ODD y arranques con T1: investigación y diseño de los contratos para Click'n'Load 2 (puerto local 9666, descifrado de enlaces CNL2, orígenes seguros), integración de Debrid y el modelo de paquetes en la base SQLite preservando 100% de compatibilidad con las descargas existentes.
> 2. Documentes los hallazgos técnicos de T1 en el log del ODD y la propuesta de schema para paquetes.
> 3. Todo sobre main, commits atómicos (≤400 líneas, Conventional Commits y sin Co-Authored-By) y cero sobreingeniería.
>
> Avisame apenas tengas T1 cerrado para autorizar la implementación de T2.

- L3 — Hallazgos de T1 (investigación, sin código).

  **Click'n'Load 2.** Contrato verificado en la [documentación oficial](https://jdownloader.org/knowledge/wiki/glossary/cnl2) y en una [implementación independiente](https://github.com/drwilly/clicknload2text). El cliente hace `POST http://127.0.0.1:9666/flash/addcrypted2` con formulario `application/x-www-form-urlencoded` y cuatro campos: `passwords` (contraseña del paquete, opcional), `source` (referer que nombra el paquete), `jk` (código JavaScript que define una función `f()` que devuelve la clave) y `crypted` (carga cifrada). Existe la variante en claro `/flash/add` con `urls`, y `/jdcheck.js` para detectar si el gestor está corriendo.

  Cifrado: AES-128-CBC con la clave y el IV iguales a los 16 bytes que representa el hexadecimal de 32 caracteres devuelto por `f()`, sin salt y sin relleno; el texto plano son URLs separadas por `\r\n` y el cliente descarta los bytes nulos. Nunca se evaluará el JavaScript: se acepta solo la forma `return '<32 hex>'` mediante una expresión estricta y se rechaza cualquier otra.

  Orígenes y superficie: se escucha únicamente en `127.0.0.1`; se valida la cabecera `Host` contra `localhost:9666`, `127.0.0.1:9666` y `[::1]:9666`, y el par debe ser de loopback. Un sitio web cualquiera puede emitir ese POST, así que la función queda apagada por defecto, exige consentimiento explícito antes de encolar y nunca arranca descargas por sí sola. Límites de entrada: cuerpo acotado, URLs solo `http`/`https`, tope de enlaces por paquete, deduplicación y rechazo de cargas que no descifren. El antecedente de [ejecución remota de código de 2010](https://seclists.org/fulldisclosure/2010/Feb/123) en la implementación original justifica ese diseño.

  Dependencias: descifrar requiere sumar `aes` y `cbc` (RustCrypto, pure Rust). Es la única incorporación de dependencias de esta fase y se decide en T3, no en T1.

  **Debrid.** Contratos por proveedor: Real-Debrid usa `POST https://api.real-debrid.com/rest/1.0/unrestrict/link` con `Authorization: Bearer <token>` y devuelve el campo `download`; AllDebrid usa `link/unlock` de su API v4 con la misma cabecera y devuelve `data.link`; TorBox expone `requestdl` sobre torrents y su flujo para hosters no está verificado, por lo que T4 no puede comprometerlo todavía. Diseño común: el token vive en el almacén de secretos existente, nunca en SQLite, en logs ni en respuestas de error; solo se aceptan URLs de los hosts oficiales sobre TLS; un fallo del proveedor produce un error mapeado y jamás un enlace inventado; se resuelve antes de encolar y no se reintenta un 4xx.

  **Licencia.** Segmenta es GPL-3.0-or-later y el material de referencia de MDM es GPL-3.0; incluir código GPL-3.0 en un proyecto GPL-3.0-or-later es compatible, con atribución y conservación de la licencia. Aun así T1 no reutiliza código de MDM: los contratos se tomaron de la documentación de cada protocolo, así que no hay obligación nueva.

  **Almacenamiento de secretos.** `crates/segmenta-daemon/src/secrets.rs` ya expone un almacén con respaldo de archivo privado; los tokens de Debrid y las contraseñas de paquetes usan esa vía y nunca columnas de texto plano.

  **Propuesta de schema para paquetes (v7).** Aditiva, sin reconstruir tablas, para conservar el 100% de las descargas existentes:

  ```sql
  CREATE TABLE IF NOT EXISTS packages (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      source TEXT NOT NULL DEFAULT 'manual',  -- 'manual' | 'cnl2' | 'gallery'
      source_ref TEXT,
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );
  ALTER TABLE downloads ADD COLUMN package_id TEXT REFERENCES packages(id) ON DELETE SET NULL;
  CREATE INDEX IF NOT EXISTS idx_downloads_package ON downloads(package_id);
  ```

  Justificación de compatibilidad: `ALTER TABLE ... ADD COLUMN` deja intactas las filas existentes y las deja con `package_id NULL`, que significa «suelta»; no se tocan CHECK ni claves foráneas de las tablas actuales; `ON DELETE SET NULL` garantiza que borrar un paquete nunca borra descargas. La migración sigue el patrón de v6 (`has_column` + guarda de idempotencia) y sube `SCHEMA_VERSION` a 7. En IPC: `package.list/create/rename/delete`, `package_id` en `download.list` y filtro por paquete; `package.delete` desagrupa, no borra. Los paquetes no guardan contraseñas: la contraseña del paquete CNL se reenvía al secreto por descarga y se descarta de memoria al terminar.

  **Riesgos que T2–T4 deben resolver, ya identificados:** el listener de CNL convive con JDownloader en el puerto 9666 (si está ocupado, la función se informa deshabilitada, sin reintentos); un paquete con miles de enlaces puede saturar la cola y necesita tope; y el popup de la extensión debe seguir funcionando cuando la captura está apagada.

- L4 — Implementación de T2 (autorizada por el PO: migración v7, soporte en el store, IPC y acciones colectivas, con pruebas de compatibilidad).
  - Store: `SCHEMA_VERSION` a 7 con paso aditivo (tabla `packages`, columna `downloads.package_id` con `ON DELETE SET NULL`, índice `idx_downloads_package`), tipos `PackageRow`, normalización de nombre (recorte, 1..=120) y métodos `create_package`, `packages`, `rename_package`, `delete_package`, `assign_package`, `package_task_ids`. `schema.sql` refleja la forma actual para una base nueva.
  - IPC: `package.list`, `package.create`, `package.rename`, `package.delete`, `package.assign`, `package.pause`, `package.resume`, `package.cancel`. Las acciones colectivas devuelven las listas afectadas y las omitidas, y nunca abortan el lote por el estado de una tarea: pausar una tarea terminal o ya pausada la omite, reanudar una terminal la omite, cancelar una completada/fallida/cancelada no reescribe su historial. Un paquete inexistente devuelve error tipado y un nombre vacío `-32602`.
  - Compatibilidad: una base v6 se migra sin reconstruir tablas y conserva sus descargas (prueba `a_v6_database_gains_packages_without_touching_its_downloads`, que arma la forma real de 0.4.0 quitando índice, tabla y columna). Borrar un paquete deja vivas sus descargas (`deleting_a_package_keeps_its_downloads`).
  - Pruebas de integración contra un daemon real en socket temporal: agrupación y listado, acciones colectivas respetando estado, borrado de paquete y errores tipados. Suite completa del daemon, `cargo fmt --check` y `cargo clippy -D warnings` en verde.
  - Hallazgo colateral corregido: `cargo clippy` marcaba un lifetime innecesario en `extract.rs` introducido en la corrección de v0.4.0 (`9667052`); se arregló en `3d0e631`. Los gates de v0.4.0 no se volvieron a correr después de aquella corrección, así que el aviso quedó visible recién ahora.
  - GUI pendiente: la agrupación visible y las acciones de paquete en la interfaz no forman parte de este commit.
