# Segmenta 0.6.0

Release de la Fase 3: red avanzada y control remoto. La base de v0.5.0 no cambia de
comportamiento salvo por lo que sigue.

## Buscador de la ventana

El botón de búsqueda ahora abre la barra. La causa era `set_visible(false)` sobre la
`GtkSearchBar`: el widget quedaba fuera del layout y el toggle activaba el modo sin que GTK
dibujara nada. La barra vive bajo el encabezado y el campo toma el foco al abrirse.

## Paquetes en la ventana

La barra lateral lista los paquetes con su nombre y cantidad de descargas, y ofrece pausar,
reanudar y eliminar sobre el seleccionado. `package.delete` viaja siempre con
`delete_file: false`: borrar un paquete nunca borra archivos.

## Debrid transparente al agregar

Si hay proveedor configurado y con token, el diálogo de alta resuelve el enlace con
`debrid.resolve` antes de encolar. Si no hay proveedor, si falla o si la respuesta no es
`https`, se descarga la URL original sin mostrar error.

## VPN con kill switch

Ajuste `vpn_interface`. Los sockets HTTP se atan al dispositivo (`SO_BINDTODEVICE`) y, si la
interfaz no existe o está caída, la descarga se rechaza **antes** de crear cualquier socket:
1080 nombre inválido, 1081 inexistente, 1082 caída. Los torrentes se rechazan con 1083 cuando
hay interfaz configurada, porque librqbit no permite atar sus sockets: permitirlos filtraría
la IP real.

## Control remoto con emparejamiento

Apagado por defecto. `segmenta remote pair` acuña un token de 32 bytes, lo muestra una sola
vez con la URL de emparejamiento y, si `qrencode` está instalado, dibuja el QR en la terminal.
En el almacén de secretos queda solo el SHA-256 del token.

`GET /api/downloads` y `POST /api/action` (`pause`, `resume`, `cancel`) exigen
`Authorization: Bearer`; no hay endpoint anónimo. `segmenta remote revoke` invalida el token
de inmediato y cierra el puerto.

## RSS

Schema **v8**: `rss_feeds` y `rss_seen_items`, aditivo, sin tocar las tablas existentes.
`rss.feed.add/list/remove` y `rss.check_now`; el daemon además sondea cada minuto los feeds
cuyo intervalo venció. El parser acepta RSS y Atom, filtra por alternativas separadas con `|`
y deduplica por GUID, de modo que un reinicio no vuelve a importar lo mismo. Las URLs de feed
se validan contra SSRF salvo que se permitan descargas locales.

## Suplantación TLS: no incluida

`tls_profile` existe como seam. En este build `chrome` se rechaza con un error que nombra la
feature faltante: la suplantación real necesita BoringSSL y su cadena de compilación, y se
decidió no arrastrarla al paquete. Ver `docs/tls-impersonation-evaluation.md`.

## Instalación

```bash
sudo apt install ./segmenta_0.6.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
```

## Limitaciones conocidas

- El control remoto habla HTTP sin TLS: es una superficie de LAN, no un reemplazo del socket
  Unix, y no limita intentos de autenticación.
- El QR se genera solo desde la CLI y solo si `qrencode` existe; no hay emparejamiento desde
  la ventana.
- Los torrentes no pueden atarse a una interfaz, y por eso quedan bloqueados bajo VPN.
- El RSS solo encola descargas HTTP/HTTPS: un `.torrent` o un magnet en el feed no crea una
  tarea de torrent.
- La sección de paquetes lista y acciona, pero no filtra la lista de descargas por paquete.
- La extracción de galerías sigue siendo sincrónica y no deduplica al reimportar.
