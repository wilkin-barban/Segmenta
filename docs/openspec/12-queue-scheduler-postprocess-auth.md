# OpenSpec 12: Scheduler de colas, post-proceso, antivirus y site logins

> **Especificación:** OPENSPEC-SEGMENTA-012
> **Versión:** 1.0.0
> **Área:** Scheduler nativo, `max_active`, hook argv de post-proceso, ClamAV/cuarentena y credenciales de sitio
>
> ## (0.2, en preparación)
>
> **Este documento no describe comportamiento entregado.** Es el contrato acordado para 0.2.0; las
> unidades S1, S2, A1, A2 y L1 de `odd/tasks/segmenta-0.2.md` están pendientes y no existirá tag
> `v0.2.0` mientras algo de aquí no esté implementado, probado y verificado con QA real en Ubuntu.
> Nada de lo aquí escrito debe citarse como «ya funciona» en README, CHANGELOG, notas de release ni
> en la GUI. Precedencia documental: 11 §1 (07 manda en métodos, settings y errores; 08 manda en el
> DDL).

---

## 1. Alcance

Entra en 0.2:

| Unidad | Entrega |
| --- | --- |
| S1 | Scheduler de colas nativo con ventanas horarias y acción al completar |
| S2 | `max_active` real por cola y liberación de slot al terminar |
| A1 | Hook argv de post-proceso con `%file%`, `%folder%`, `%name%` y timeout |
| A2 | ClamAV integrado opcional, cuarentena y `ERR_ANTIVIRUS_FOUND` |
| L1 | Site logins HTTP Basic + GNOME Keyring en el daemon y su UI |

No entra: Digest ni otros esquemas de autenticación, categorías automáticas, adaptadores de sniffer
fuera de YouTube, tiendas de extensiones, ni ejecución del scheduler con el daemon apagado (ver §10).

Regla transversal: ninguna unidad se marca `done` si su contrato en este documento no está actualizado
en el mismo commit que el código.

---

## 2. Scheduler de colas

**Decisión:** un único loop `tokio` dentro del daemon (`QueueScheduler`), arrancado desde el servicio
junto al resto de tareas. No hay scheduler en la GUI ni en el CLI.

- **Tick:** 30 s. En tests se controla con reloj pausado (`tokio::time::pause`); no se duerme en tiempo
  real ni se depende de la hora del sistema para las pruebas.
- **Fuente de verdad:** el ajuste persistido por cola (`queue_schedule:<name>`) y, para la cola `main`,
  los campos legacy `schedule_start`/`schedule_stop`. La GUI no mantiene estado propio: lee y escribe
  por IPC (`queue.list`, `queue.schedule`).
- **Ventana horaria:** hora local del usuario; una ventana con hora de inicio posterior a la de fin
  cruza medianoche y se interpreta como tal. Un valor inválido se rechaza en `queue.schedule` con
  error; no se guarda un schedule que el scheduler no pueda evaluar.
- **Estado por cola:** ventana abierta o cerrada, tareas activas y pendientes. El scheduler no mueve
  `end_offset` ni toca la tabla de segmentos: sólo decide qué tarea puede arrancar.
- **Arranque centralizado:** todo pendiente arranca por el mismo camino
  (`start_pending_for_queue`), que consulta ventana y `max_active` **antes** de llamar a
  `start_task`. No se duplica esa lógica en GUI, CLI ni en el handler RPC.
- **Despertar:** al completar, fallar o cancelar una tarea, el scheduler se notifica para arrancar la
  siguiente en la misma cola; no se espera al siguiente tick para liberar el turno.
- **Concurrencia:** el mutex de tareas en ejecución no se retiene durante `await`; el scheduler no
  bloquea el resto del daemon mientras evalúa.
- **`action_on_complete`:** `none` (por defecto), `suspend`, `power_off`, `exit`. Se ejecuta **sólo**
  cuando la cola queda vacía (sin pendientes ni activas), una única vez por vaciado. `exit` usa el
  apagado ordenado que ya existe; `suspend`/`power_off` requieren permiso del sistema y no se prueban
  de forma destructiva en CI ni en el QA de 0.2.

---

## 3. `max_active` por cola

- `max_active` es un límite **por cola**, aplicado por el daemon en el punto único de arranque de §2.
- Una tarea en estado terminal (completada, fallida o cancelada) libera su slot de inmediato; una
  pausada no cuenta como activa.
- `max_active` no reduce el paralelismo dentro de una tarea: el número de conexiones de una descarga lo
  decide el motor (06).
- Cambiar `max_active` no cancela tareas ya activas; el nuevo valor se aplica al siguiente arranque.
  Bajar el límite por debajo de las activas actuales deja la cola en deuda hasta que terminen.
- La GUI muestra el límite y el conteo leídos por IPC; no lo recalcula por su cuenta.

---

## 4. Hook argv de post-proceso

- **Representación:** `postprocess_command` sigue siendo una lista de argumentos (`Vec<String>`), no un
  string de shell. Nunca se invoca un shell (`sh -c`, backticks, `$(...)` no se interpretan).
- **Sustituciones:** sólo tres placeholders, sustituidos como argumento completo:
  - `%file%` → ruta absoluta del archivo final;
  - `%folder%` → directorio destino de la tarea;
  - `%name%` → nombre del archivo.
  Un placeholder desconocido deja la lista inválida y el hook no se ejecuta.
- **Ejecución:** `std::process::Command` con argv explícito, stdout/stderr acotados (no se guardan
  volcados completos) y timeout configurable. Un hook que excede el timeout se mata y se registra.
- **Resultado:** se persisten `postprocess_exit` y `postprocess_message` en la tarea. Un código de
  salida distinto de 0 **no** convierte una descarga correcta en fallida salvo que el usuario lo
  configure; se registra y se muestra como advertencia.
- **Sin hook configurado:** la fase se omite y la tarea pasa a `completed`, igual que hoy.
- Un fallo de hook nunca borra el archivo original.

---

## 5. Antivirus y cuarentena

**Decisión:** ClamAV integrado y **opcional**. El `.deb` no lo embebe ni lo declara en `Depends` (10 §2.0).

Settings:

| Setting | Valor por defecto | Efecto |
| --- | --- | --- |
| `antivirus_enabled` | `false` | Activa la fase de escaneo obligatoria antes de `completed` |
| `antivirus_timeout_secs` | `120` | Timeout del escaneo; al excederse, el escaneo falla |
| `antivirus_quarantine_dir` | `$XDG_DATA_HOME/segmenta/quarantine` | Destino de los archivos infectados |

- **Motor:** se usa `clamdscan` si existe (habla con `clamd` por socket) y `clamscan` como alternativa;
  nunca `sh -c` ni una cadena de comando del usuario.
- **Estados:** el flujo de cierre es `finalizing` → `verifying` → escaneo/hook → `completed` o
  cuarentena/fallo. El archivo no se publica como `completed` antes de terminar la fase.
- **Infección:** el archivo se **mueve** a `antivirus_quarantine_dir` (misma unidad cuando es posible;
  si no, copia verificada y borrado del original), la tarea queda `failed` con
  `ERR_ANTIVIRUS_FOUND` (código nuevo `1046`) y se persiste `quarantine_path` junto con el resultado.
  El archivo **nunca** queda en Descargas ni aparece como completado.
- **Motor ausente con antivirus activo:** la tarea queda `failed` con `ERR_ANTIVIRUS_UNAVAILABLE`; no se
  publica un `completed` que nadie verificó.
- **Limpio:** sigue el flujo normal (hook de post-proceso, si está configurado, y luego `completed`).
- **Fallo del escaneo** (timeout, error del binario, código inesperado): se registra en
  `postprocess_message` y la tarea no se declara `completed`.
- **UI (U1):** sección Antivirus en Preferencias con el interruptor y los tres valores, y estado «En
  cuarentena» con la ruta. El comando de escaneo no se presenta como secreto porque no lo es; la
  contraseña de un site login tampoco se muestra nunca (§6).

---

## 6. Site logins (HTTP Basic + GNOME Keyring)

**Decisión:** autenticación **HTTP Basic** por dominio, con la contraseña en GNOME Keyring vía
`secret-tool`. **Sin Digest** y sin otros esquemas en 0.2.

Métodos RPC nuevos (los únicos que este documento autoriza; el catálogo es de 07):

| Método | Entrada | Respuesta | Reglas |
| --- | --- | --- | --- |
| `site_login.list` | — | entradas sin `password` | Nunca devuelve el secreto, sólo dominio y usuario |
| `site_login.set` | dominio, usuario, password | entrada sin `password` | Guarda `username` en SQLite y la password en el keyring |
| `site_login.remove` | dominio | confirmación | Borra la entrada de SQLite y el secreto del keyring |

- **Almacenamiento:** `username` en la tabla `site_logins` (08); password en el keyring con alias
  `site/<domain>`. Sin keyring disponible, `site_login.set` falla con `1031`
  (`ERR_KEYRING_UNAVAILABLE`) y **no** crea la entrada: nunca se escribe la password en SQLite ni en
  un archivo plano.
- **Validación:** dominio exacto, sin comodines ni subdominios implícitos; URL `http`/`https`; sin
  credenciales embebidas en la URL (`user:pass@host` se rechaza).
- **Inyección:** al añadir, iniciar o probar una URL cuyo host tenga login, el daemon añade
  `Authorization: Basic …` como cabecera sensible. Una cabecera explícita de la propia tarea gana
  siempre sobre el login de sitio.
- **Redirects:** se respeta la regla existente de no reenviar credenciales en un redirect
  cross-origin; el header se descarta al cambiar de origen.
- **401 sin login configurado:** se mantiene `ERR_AUTH_REQUIRED`. No hay fallback silencioso a Basic ni
  reintento automático con credenciales que el usuario no configuró.
- **Secretos fuera de logs:** la password no aparece en logs, trazas, eventos ni en la respuesta de
  `settings.get`; las pruebas de L1 lo verifican explícitamente.
- **UI (U1):** página «Contraseñas de sitios» con añadir, editar y borrar, y diálogo de contraseña.
- **Limitación documentada:** un servidor que sólo ofrece Digest no se puede usar en 0.2. Se documenta
  como límite, no como bug abierto ni como promesa.

---

## 7. Persistencia (schema v4)

El DDL normativo es el de `08-persistence-recovery.md`; aquí sólo se fija qué necesita este contrato:

- Migración a `user_version = 4`, idempotente, con `site_logins` (dominio, usuario; sin secreto) y los
  campos de cuarentena/post-proceso de la tarea: `quarantine_path`, `postprocess_exit`,
  `postprocess_message`.
- `DownloadRecord`/`DownloadView` exponen esos campos; la GUI los consume por IPC, nunca por SQLite
  directo.
- Los estados `retrying` y `finalizing` existen en el daemon y la GUI debe pintarlos (deuda de U1 en el
  ODD).

---

## 8. Errores

| Código | Nombre | Cuándo |
| --- | --- | --- |
| `1046` | `ERR_ANTIVIRUS_FOUND` | El escaneo detectó infección; el archivo quedó en cuarentena |
| `1047` | `ERR_ANTIVIRUS_UNAVAILABLE` | `antivirus_enabled=true` y no hay `clamdscan` ni `clamscan` utilizable |
| `1031` | `ERR_KEYRING_UNAVAILABLE` | `site_login.set` sin keyring disponible |
| — | `ERR_AUTH_REQUIRED` | El servidor pide credenciales y no hay login de sitio configurado |

Los números que no aparecen aquí todavía no están asignados; 07 es el catálogo normativo y debe
recibirlos antes de implementar el error correspondiente. No se inventan códigos en el código.

---

## 9. Pruebas obligatorias

- **Scheduler:** ventana abre y cierra con reloj pausado; una tarea terminada libera slot; `max_active`
  nunca se supera; `queue.schedule` inválido devuelve error; la acción al completar se ejecuta una sola
  vez y sólo con la cola vacía; nada arranca si el daemon no está activo.
- **Post-proceso:** `%file%` como argumento, archivo con espacios y caracteres de shell, ausencia de
  shell, timeout, código de salida distinto de 0.
- **Antivirus:** fakes de `clamscan` y `clamdscan` para limpio, infectado, ausente y timeout; el archivo
  infectado queda en cuarentena y la tarea en `failed` con `1046`; nunca `completed`.
- **Site logins:** Basic correcto contra un servidor local, password incorrecta, sin keyring (`1031`),
  secreto fuera de logs y de `settings.get`, credenciales no enviadas en redirect externo, cabecera
  explícita de la tarea prevalece.
- **Persistencia:** migración v3 → v4 idempotente y recuperación de una tarea en `finalizing`/
  `verifying` tras `SIGKILL`.

---

## 10. Límites explícitos

- El scheduler sólo corre mientras el daemon esté activo; la activación por socket lo despierta, no
  adelanta el reloj.
- `suspend` y `power_off` dependen de permisos del sistema y no se prueban de forma destructiva en CI;
  el QA de 0.2 usa ventanas cortas y fixtures locales.
- ClamAV es externo y opcional: su calidad depende de firmas actualizadas que el paquete no gestiona.
- Basic sin Digest no cubre todos los servidores.
- La cuarentena vive en `$XDG_DATA_HOME/segmenta/quarantine` y no se limpia automáticamente en 0.2; el
  usuario decide cuándo vaciarla.
