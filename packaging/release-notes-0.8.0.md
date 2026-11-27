# Segmenta 0.8.0

Red e inmunidad: el tráfico deja de escapar del túnel, los nombres se resuelven cifrados, y el
motor HTTP se adapta cuando un servidor pide menos. La versión instalada sigue siendo 0.7.0
hasta que este documento se publique; nada de lo que sigue altera el formato de la base salvo
una tabla nueva.

## Relay interno y herramientas externas

`yt-dlp`, `gallery-dl`, los hooks y el antivirus ya no abren sus propios sockets contra la red
del sistema. Un relay loopback autenticado recibe sus peticiones y las abre atadas a la
interfaz de la VPN. Antes de cada conexión el relay vuelve a comprobar que la interfaz exista y
esté arriba: si no lo está, responde `503` y no hay reintento ciego. Las variables de proxy
heredadas se descartan, así que un `HTTP_PROXY` del entorno no puede secuestrar la descarga
fuera del túnel.

`ffmpeg` no se enruta: en Segmenta solo concatena archivos locales y no abre red. Está escrito
para que nadie asuma lo contrario.

## Torrents sobre VPN

Los torrents dejaron de rechazarse. La sesión de librqbit se crea con `bind_device_name`,
sin descubrimiento local y sin listener entrante ni UPnP. Como el bind no cubre una conexión
abierta antes de que el túnel caiga, el planificador pausa las tareas en vuelo cuando la
interfaz deja de estar arriba. La sesión es única por proceso: cambiar de interfaz exige
reiniciar el daemon.

## DNS cifrado

`dns_mode` acepta `system`, `doh` y `dot`. DoH habla RFC 8484 con Cloudflare, Quad9, Google o
una URL propia, y sus respuestas pasan por las mismas guardas anti-SSRF que el resolvedor del
sistema: un endpoint hostil no puede apuntar una descarga a loopback ni a la LAN. **DoT no
está incluido en este build** y devuelve un error que lo dice; no se simula. PAC tampoco: haría
falta un intérprete de JavaScript que el proyecto no incorpora.

## Reglas por sitio

Una regla por dominio puede fijar user agent, tope de conexiones y headers. Los headers
sensibles se guardan en el almacén de secretos y en la base queda solo un alias que nunca sale
del daemon. La página de preferencias lista las reglas sin mostrar ni valores ni alias, y el
selector de proveedor DNS solo está activo cuando el modo es DoH.

## Motor adaptativo

- **Límite por host**: un `429` o `503` divide el límite a la mitad, nunca por debajo de una
  conexión; cada período de calma devuelve un slot hasta el valor inicial.
- **Tope de 128 conexiones**, y ninguna regla ni ajuste lo amplía.
- **Evicción de workers lentos**: un worker por debajo del 25 % del más rápido, con al menos
  `MIN_SPLIT_SIZE` pendiente, se reemplaza por una conexión nueva sin mover los offsets del
  disco.
- **Streams sin `Content-Length`**: se crean sparse y `download.keep` publica lo que llegó al
  disco, porque ese stream no se puede reanudar y descartarlo tiraba lo ya transferido.

## Corrección de una prueba inestable

`snapshot_of_exactly_the_page_size_paints_no_notice` esperaba 500 ms fijos a la respuesta de
conteo y fallaba en máquina cargada. Ahora espera la condición con un límite de 10 s: un total
equivocado deja el aviso pintado y la prueba sigue fallando, pero por el motivo correcto.
