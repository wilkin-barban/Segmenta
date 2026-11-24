# Segmenta 0.5.0

Release de la Fase 2: ecosistema web, Debrid y paquetes. La base de v0.4.0 no cambia de
comportamiento salvo por lo que sigue.

## Paquetes y acciones colectivas

La base sube a **schema v7** con una migración aditiva: tabla `packages`, columna
`downloads.package_id` con `ON DELETE SET NULL` e índice. Las descargas existentes quedan con
`package_id NULL` —«sueltas»— y no se reconstruye ninguna tabla, así que actualizar no mueve
ni un byte de las tareas previas.

Métodos nuevos: `package.list`, `create`, `rename`, `delete`, `assign`, y las acciones
colectivas `package.pause`, `package.resume`, `package.cancel`. Cada una devuelve las tareas
afectadas y las omitidas, y nunca aborta el lote por el estado de una tarea: pausar una tarea
terminal la omite, y cancelar una completada no reescribe su historial. Borrar un paquete
**nunca** borra descargas.

## Click'n'Load 2

El daemon puede escuchar en `127.0.0.1:9666` para importar paquetes desde sitios que ofrecen
Click'n'Load. Está **apagado por defecto** (`cnl2_enabled`).

- Descifra AES-128-CBC tal como lo especifica el protocolo: clave e IV iguales, sin salt, sin
  relleno, con tolerancia a remitentes que usen PKCS#7.
- **Nunca evalúa JavaScript**: de `jk` solo acepta 32 hexadecimales o `return '<hex>'` sin
  sentencias extra.
- Solo atiende a un par de loopback, valida `Host` contra su propio puerto, corta en 64 KiB
  de cuerpo y 1000 enlaces, y acepta únicamente `http`/`https`.
- Si el puerto está ocupado —JDownloader suele tenerlo— registra la degradación y el daemon
  sigue funcionando sin reintentos ni pánico.

Habilitarlo requiere reiniciar el daemon: el puerto se abre una sola vez al arrancar.

## Debrid: Real-Debrid y AllDebrid

Adaptadores en `segmenta-core` que describen la petición y leen la respuesta, sin red en las
pruebas. El token vive en el almacén de secretos (`debrid:<proveedor>`), **nunca** en SQLite,
en logs ni en respuestas IPC; `debrid.status` informa si hay token pero jamás lo devuelve.

Del proveedor solo se acepta una URL `https://` sin credenciales embebidas: cualquier otra
cosa se rechaza como respuesta inválida. Los fallos se traducen a códigos estables
(`ERR_DEBRID_*`, IPC 1060–1065) con una frase fija, sin filtrar el cuerpo del proveedor.

TorBox está reconocido pero **no implementado**: su flujo para hosters quedó sin verificar y
devolver «no soportado» es preferible a inventar un contrato.

## Popup de descargas en Chrome y Firefox

El popup muestra las descargas recientes con su estado y ofrece acciones directas: pausar y
cancelar mientras corre, reanudar y cancelar en pausa. El host nativo ya reenviaba los métodos
`download.*`, así que no hizo falta ampliar su superficie; el popup usa dos mensajes nuevos,
`segmenta.list` y `segmenta.action`, validados contra listas blancas.

## gallery-dl

`gallery.status` detecta el binario y `gallery.add` extrae una galería y agrupa los archivos en
un paquete con `source: gallery`. Corre por argv, sin shell, con plazo de 110 s y captura
acotada; si el binario falta, la importación se rechaza con código propio y el daemon sigue
igual. Códigos: 1070 ausente, 1071 vacío, 1072 fallo.

## Instalación

```bash
sudo apt install ./segmenta_0.5.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
```

## Limitaciones conocidas

- La resolución automática de Debrid dentro de `download.add` no está conectada: hoy el punto
  de entrada es `debrid.resolve`.
- No se probó ninguna llamada real a un proveedor Debrid (no hay credenciales en el build).
- No hay superficie de GUI para paquetes, Click'n'Load ni galerías.
- La extracción de galerías es sincrónica (110 s) y no deduplica al reimportar.
- La contraseña de un paquete CNL se parsea pero todavía no se adjunta a las descargas.
- La prioridad inicial de torrents y la inhibición real de suspensión siguen sin prueba
  automatizada contra el servicio del sistema.
