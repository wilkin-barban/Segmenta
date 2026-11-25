# Automatización de la fase 1: MCP, extracción, inhibición y torrents

Estado: implementado y publicado; el servidor MCP viaja en el `.deb` desde v0.4.0.
Contrato y evidencia por tarea: [`odd/tasks/segmenta-v0.4.0-automatizacion.md`](../odd/tasks/segmenta-v0.4.0-automatizacion.md).

## Servidor MCP por stdio

`/usr/bin/segmenta-mcp` es un puente ligero: habla JSON-RPC por líneas sobre stdin/stdout (el
transporte MCP estándar) y reenvía cada herramienta al daemon por su socket Unix:
`SEGMENTA_SOCKET`, si está definido; si no, `$XDG_RUNTIME_DIR/segmenta/daemon.sock`; y como último
recurso `/tmp/segmenta-<uid>/daemon.sock`. No abre puertos, no guarda estado, no inicia el daemon y
no modifica la configuración de ningún cliente: se declara a mano en el cliente MCP.

Cada herramienta usa el método IPC existente (`download.list`, `download.add`, `download.pause`,
`download.resume`, `download.delete`) sin métodos arbitrarios.

### Configuración del cliente

El paquete instala el binario en `/usr/bin/segmenta-mcp`. Agregalo a la configuración del cliente
(`~/.config/Claude/claude_desktop_config.json`, `~/.cursor/mcp.json` o el equivalente):

```json
{
  "mcpServers": {
    "segmenta": {
      "command": "/usr/bin/segmenta-mcp"
    }
  }
}
```

Reiniciá el cliente: el servidor aparece con las cinco herramientas de abajo.

### Herramientas y parámetros

#### `segmenta_list`

Lista las descargas con el mismo contrato que `download.list`.

| Parámetro | Tipo | Obligatorio | Descripción |
| --- | --- | --- | --- |
| `filter` | string | no | `all` (por defecto), `active`, `paused`, `pending`, `completed`, `failed` o `cancelled` |
| `limit` | integer | no | Filas a devolver; el daemon acota entre 1 y 1000, con 100 por defecto |
| `offset` | integer | no | Filas a saltear, para paginar; 0 por defecto |

#### `segmenta_add`

Agrega una descarga y devuelve `task_id`, estado, nombre y ruta final.

| Parámetro | Tipo | Obligatorio | Descripción |
| --- | --- | --- | --- |
| `url` | string | **sí** | URL `http` o `https` |
| `destination_dir` | string | no | Directorio absoluto; si falta se usa el de la configuración |
| `filename` | string | no | Nombre sugerido; si falta se deduce del servidor |
| `connections` | integer | no | Conexiones para esa tarea; el daemon acota entre 1 y 32 |
| `start_immediately` | boolean | no | Arrancar al crearla; `true` por defecto |
| `queue_name` | string | no | Cola destino; `main` por defecto |
| `priority` | integer | no | Prioridad, mayor primero; 0 por defecto |

#### `segmenta_pause`

| Parámetro | Tipo | Obligatorio | Descripción |
| --- | --- | --- | --- |
| `task_id` | string | **sí** | Tarea a pausar |

#### `segmenta_resume`

| Parámetro | Tipo | Obligatorio | Descripción |
| --- | --- | --- | --- |
| `task_id` | string | **sí** | Tarea a reanudar o reintentar |

#### `segmenta_delete`

| Parámetro | Tipo | Obligatorio | Descripción |
| --- | --- | --- | --- |
| `task_id` | string | **sí** | Tarea a eliminar |
| `delete_file` | boolean | no | Borrar también el archivo del disco; `false` por defecto, y el servidor lo envía explícito porque el daemon exige el campo |

### Límites y errores

Frames de 1 MiB, 128 respuestas del daemon por llamada y timeouts de 30 s por operación de socket.
Los diagnósticos van solo a stderr, así que stdout queda limpio para el protocolo. Los errores del
daemon se reportan como error de herramienta (`isError: true`) sin filtrar el detalle interno ni
devolver la credencial de una URL.

Riesgo operativo: cualquier cliente conectado puede borrar tareas y archivos. Conectá solo clientes
confiables; el daemon sigue autorizando por UID del socket.

## Extracción de archivos

Ajustes (todos apagados por defecto):

- `extract_archives` (`false`): extrae un archivo terminado en su propia carpeta.
- `extract_remove_source` (`false`): borra el comprimido **solo** si la extracción se completó.

Formatos: los que resuelve la herramienta externa (`zip`, `7z`, `rar` y volúmenes multiparte).
Dependencia del sistema: `7z` (`p7zip-full`) y, si falta, `unzip` para zip. Sin ninguna de las dos,
la fase se omite con un mensaje y la descarga sigue completada.

Contraseñas: variable de entorno `SEGMENTA_EXTRACT_PASSWORD` en el proceso del daemon. No se guarda
en la base de datos.

Protección: se listan las entradas antes de extraer y se rechaza el archivo completo si alguna ruta
es absoluta o contiene `..`; la extracción escribe en una carpeta temporal dentro del destino, que se
descarta si aparece un enlace simbólico, y solo después se mueve lo extraído, sin sobrescribir nada
existente.

## Inhibición de suspensión

Ajuste `inhibit_sleep` (`true` por defecto). Mientras hay tareas en vuelo, el daemon mantiene el
bloqueo `sleep` a través de `systemd-inhibit` y lo libera al pausar, completar, cancelar o fallar la
última tarea. Sin `systemd-inhibit` o sin logind, la inhibición se omite en silencio: las descargas
no dependen de ella.

## Torrents con prioridad inicial

Ajuste `torrent_sequential` (`false` por defecto). Al activarlo, el daemon mantiene abierto un
`FileStream` de librqbit sobre el primer archivo mientras dura la tarea, lo que prioriza las piezas
iniciales y permite empezar a reproducir antes.

Límite real del backend: librqbit 9.1.0 no expone un interruptor de descarga secuencial para el
torrent completo. Sin ese stream abierto, las piezas llegan en el orden que ofrecen los pares y el
orden de reproducción no está garantizado.

## Dependencias

- `7z` o `unzip` para extracción (opcional).
- `systemd-inhibit` y logind para inhibición (opcional).
- Ninguna dependencia nueva de Rust: extracción e inhibición usan procesos externos con argv, sin
  shell.

## Paquetes (schema v7)

La base agrega la tabla `packages` y `downloads.package_id` de forma aditiva: las descargas que
ya existían quedan sueltas (`package_id NULL`) y no se reconstruye ninguna tabla.
`ON DELETE SET NULL` garantiza que borrar un paquete nunca borre sus descargas.

Métodos: `package.list`, `package.create`, `package.rename`, `package.delete`,
`package.assign` y las acciones colectivas `package.pause`, `package.resume` y
`package.cancel`. Cada acción devuelve las tareas afectadas y las omitidas; una tarea en estado
terminal se omite en lugar de reescribir su historial, y el lote nunca se aborta por una tarea.

## Click'n'Load 2

Ajuste `cnl2_enabled`, apagado por defecto. Con el listener activo el daemon atiende
`POST /flash/addcrypted2` y `POST /flash/add` en `127.0.0.1:9666`, más `GET /flash/check` y
`GET /jdcheck.js` para la detección desde el sitio.

Descifrado AES-128-CBC con clave e IV iguales, sin salt y sin relleno (se tolera PKCS#7). De
`jk` solo se aceptan 32 hexadecimales o un `return '<hex>'` sin sentencias extra: **nunca** se
evalúa JavaScript. El peer debe ser loopback, `Host` se valida contra el puerto propio, el
cuerpo se corta en 64 KiB y el paquete en 1000 enlaces. Solo se aceptan URLs `http`/`https`.

Si el puerto está ocupado —JDownloader es el caso habitual— el daemon registra
`click'n'load 2 disabled: the port is not available` y sigue sirviendo IPC, sin reintentos.
Cambiar el ajuste requiere reiniciar el daemon.

Contraseña de paquete: se parsea del formulario, pero todavía no se adjunta a las descargas.

## Debrid: Real-Debrid y AllDebrid

Ajuste `debrid_provider` (`none` por defecto) y el token en el almacén de secretos bajo
`debrid:<proveedor>`; nunca en SQLite, en logs ni en respuestas IPC.

- `debrid.status` — proveedor, si está configurado y si hay token. Nunca el token.
- `debrid.token.set` / `debrid.token.clear`
- `debrid.resolve` — cambia un enlace de hoster por el directo del proveedor.

Del proveedor solo se acepta una URL `https://` sin credenciales embebidas. Los fallos usan
códigos estables 1060–1065 con una frase fija, sin filtrar el cuerpo del proveedor. TorBox está
reconocido pero devuelve «no soportado»: su flujo para hosters no está verificado.

La resolución **no** está conectada a `download.add` todavía.

## Popup de descargas

El popup lista las descargas recientes y ofrece acciones según el estado: pausar y cancelar
mientras corre, reanudar y cancelar en pausa. Usa dos mensajes nuevos, `segmenta.list` y
`segmenta.action`, con filtro y acciones en lista blanca y límite de 1 a 50 filas. El host
nativo ya reenviaba los métodos `download.*`; la extensión no tiene acceso a `debrid.*`.

Se refresca al abrir y con el botón «Actualizar»: no hay suscripción a eventos, y no muestra
velocidad ni ETA.

## gallery-dl

`gallery.status` informa si el binario está disponible y `gallery.add {url}` extrae una galería
y agrupa los archivos en un paquete con `source: gallery`. La detección usa
`SEGMENTA_GALLERY_PATH` o `PATH`.

Corre por argv, sin shell, con plazo de 110 s y captura acotada; los archivos que ya existían en
el destino no se cuentan. Códigos: 1070 binario ausente, 1071 sin archivos, 1072 fallo. Un fallo
nunca detiene el daemon.

Límites: es sincrónico, así que una galería muy grande necesita partirse; no deduplica al
reimportar; y no hay superficie de GUI ni de extensión para lanzarla.

## Limitaciones conocidas

- La extracción no verifica firmas ni contenido; solo la contención de rutas.
- Los timeouts del servidor MCP son por operación de E/S, no un plazo total de la llamada.
- Un frame sobredimensionado termina el proceso MCP en lugar de devolver un error de protocolo.
- La prioridad inicial de torrents no se verificó con un torrent real en la suite automatizada.
- La inhibición se verificó en el host con una descarga real; no hay prueba automatizada de logind.
