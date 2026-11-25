# Segmenta 0.7.0

Release de controles de la ventana: las funciones de las fases 2 y 3 se manejan desde la GUI, y
el buscador queda arreglado. La base de v0.6.0 no cambia de comportamiento salvo por lo que
sigue.

## Buscador

El clic en la lupa abre la barra **y pone el cursor en el campo**. Antes el clic activaba el
modo de búsqueda pero el foco quedaba en la lista, así que escribir no filtraba nada; solo
`Ctrl+F` funcionaba. Ahora el botón y la acción comparten el mismo camino, Escape cierra la
barra y devuelve el foco a la lista.

## Paquetes

La barra lateral lista los paquetes con su cantidad de descargas y permite pausar, reanudar y
eliminar el seleccionado. Eliminar un paquete nunca borra archivos.

## Debrid transparente

Con proveedor configurado y token, el diálogo de alta resuelve el enlace antes de encolar. Ante
cualquier fallo se descarga la URL original, sin error visible.

## VPN en el encabezado

Una píldora muestra el estado real de la interfaz: **VPN activa**, **VPN caída**, **VPN
ausente** o **VPN mal configurada**, y desaparece cuando no hay ninguna configurada. El estado
lo calcula el daemon con la misma comprobación que usa el kill switch, así que la ventana no
puede decir “protegido” mientras las descargas están bloqueadas. Al pulsarla se abren las
preferencias.

## Emparejamiento remoto

El diálogo muestra el estado del servidor y permite emparejar y revocar. Al emparejar se
muestran la URL, el token **una sola vez** con su advertencia, y un QR generado en Rust puro
—sin `qrencode` ni archivos temporales—. Revocar invalida el token y cierra el puerto.

## Suscripciones RSS

Panel con nombre, URL, destino y último sondeo de cada feed, alta con nombre, URL, filtro y
destino, baja de la seleccionada y **Comprobar ahora**. Cuando una comprobación falla, el panel
dice cuántos feeds no se pudieron leer y con qué código, en lugar de mostrar una lista vacía.
El daemon informa esos fallos en la respuesta de `rss.check_now`.

## Instalación

```bash
sudo apt install ./segmenta_0.7.0-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
```

## Limitaciones conocidas

- La sección de paquetes lista y acciona, pero no filtra la lista de descargas por paquete.
- El panel RSS no edita suscripciones y el estado de error no sobrevive al reinicio.
- El emparejamiento remoto funciona sobre HTTP sin TLS y el QR se dibuja solo desde la ventana
  (la CLI depende de `qrencode`, que no se instala por defecto).
- Torrentes bloqueados bajo VPN, RSS solo para descargas HTTP/HTTPS y suplantación TLS no
  incluida: siguen igual que en 0.6.0.
