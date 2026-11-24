# Segmenta 0.4.0

Release de la Fase 1: automatización, extracción y asistentes. La base v0.3.1 no cambia de
comportamiento salvo por lo que sigue.

## Servidor MCP por stdio

`segmenta-mcp` habla JSON-RPC por líneas sobre stdin/stdout y reenvía al daemon por su socket Unix,
con la misma resolución de ruta que el CLI. Expone `segmenta_list`, `segmenta_add`,
`segmenta_pause`, `segmenta_resume` y `segmenta_delete` sobre los métodos IPC existentes.

Límites: frames de 1 MiB, 128 respuestas del daemon por llamada y 30 s por operación de socket. Los
diagnósticos van solo a stderr, para no corromper el protocolo. Los errores del daemon se reportan
como error de herramienta sin filtrar el detalle interno.

Como cualquier cliente conectado puede borrar tareas y archivos, solo deben conectarse clientes
confiables; el daemon sigue autorizando por UID del socket.

## Extracción segura de archivos

Ajustes nuevos, ambos apagados por defecto: `extract_archives` y `extract_remove_source`. La fase
corre al finalizar una descarga, usa `7z` y, si falta, `unzip`, y nunca cambia el estado de una
descarga correcta.

Protección de rutas: se listan las entradas antes de extraer y se rechaza el archivo completo si
alguna es absoluta o contiene `..`; la extracción escribe en una carpeta temporal dentro del destino,
que se descarta si aparece un enlace simbólico, y solo entonces se mueve lo extraído, sin sobrescribir
nada existente. El comprimido original se conserva salvo que se pida lo contrario y la extracción
haya terminado bien. La contraseña, cuando hace falta, se toma de `SEGMENTA_EXTRACT_PASSWORD` y no se
guarda en la base de datos.

## Inhibición de suspensión

Ajuste `inhibit_sleep`, activo por defecto. Mientras hay tareas en vuelo el daemon mantiene el
bloqueo `sleep` con `systemd-inhibit` y lo libera al pausar, completar, cancelar o fallar la última
tarea. Sin logind o sin el binario, la inhibición se omite en silencio y las descargas no se ven
afectadas.

## Prioridad inicial de torrents

Ajuste `torrent_sequential`. Al activarlo se mantiene abierto un `FileStream` de librqbit sobre el
primer archivo, lo que prioriza las piezas iniciales y permite empezar a ver antes. librqbit 9.1.0 no
ofrece un interruptor de descarga secuencial del torrent completo, así que con el ajuste apagado el
orden de las piezas no está garantizado.

## Pruebas

Suite completa en verde: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace --exclude segmenta-gui` y `xvfb-run -a cargo test -p segmenta-gui`. Se suman
la suite de contrato del servidor MCP, siete pruebas de extracción con herramienta simulada, tres de
inhibición y la cobertura de los ajustes nuevos.

## Limitaciones conocidas

- La extracción no verifica firmas ni contenido: solo contiene las rutas.
- Si falla la publicación de una entrada ya movida, puede quedar salida parcial en el destino.
- Los timeouts del servidor MCP son por operación de E/S, no un plazo total de la llamada.
- La prioridad inicial de torrents no se verificó con un torrent real en la suite automatizada.
- La inhibición se verificó en el host con una descarga real; no hay prueba automatizada de logind.

## Instalación

```bash
sudo apt install ./segmenta_0.4.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
```

El paquete incluye las extensiones para Chromium y Firefox y el binario `segmenta-mcp`; las
extensiones también están disponibles por separado como `segmenta-extension-0.4.0.zip` y
`segmenta-firefox-0.4.0.zip`.
