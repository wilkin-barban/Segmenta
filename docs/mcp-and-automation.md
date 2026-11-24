# Automatización de la fase 1: MCP, extracción, inhibición y torrents

Estado: implementado en `main` sobre v0.3.1 (release v0.4.0 pendiente de publicación).
Contrato y evidencia por tarea: [`odd/tasks/segmenta-v0.4.0-automatizacion.md`](../odd/tasks/segmenta-v0.4.0-automatizacion.md).

## Servidor MCP por stdio

`segmenta-mcp` habla JSON-RPC por líneas sobre stdin/stdout y reenvía al daemon por su socket Unix:
`SEGMENTA_SOCKET`, si está definido; si no, `$XDG_RUNTIME_DIR/segmenta/daemon.sock`; y como último
recurso `/tmp/segmenta-<uid>/daemon.sock`. No inicia el daemon ni modifica la configuración de
ningún cliente: se declara a mano en el cliente MCP.

Herramientas: `segmenta_list`, `segmenta_add`, `segmenta_pause`, `segmenta_resume` y
`segmenta_delete`. Cada una usa el método IPC existente (`download.list`, `download.add`,
`download.pause`, `download.resume`, `download.delete`) sin métodos arbitrarios. `segmenta_delete`
envía `delete_file: false` cuando el cliente no lo indica; con `delete_file: true` el daemon borra
también el archivo.

Límites: frames de 1 MiB, 128 respuestas del daemon por llamada y timeouts de 30 s por operación de
socket. Los diagnósticos van solo a stderr. Los errores del daemon se reportan como error de
herramienta sin filtrar el detalle interno.

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

## Limitaciones conocidas

- La extracción no verifica firmas ni contenido; solo la contención de rutas.
- Los timeouts del servidor MCP son por operación de E/S, no un plazo total de la llamada.
- Un frame sobredimensionado termina el proceso MCP en lugar de devolver un error de protocolo.
- La prioridad inicial de torrents no se verificó con un torrent real en la suite automatizada.
- La inhibición se verificó en el host con una descarga real; no hay prueba automatizada de logind.
