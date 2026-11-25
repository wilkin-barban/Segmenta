# Controles de la ventana

Estado: v0.7.0. Contrato y evidencia: [`odd/tasks/segmenta-v0.7.0-gui-controles.md`](../odd/tasks/segmenta-v0.7.0-gui-controles.md).

## Buscador

El botón de la lupa abre la barra y **mueve el cursor al campo**. El clic y `Ctrl+F` comparten
el mismo camino, así que el botón y la acción no pueden desincronizarse; Escape cierra la barra
y devuelve el foco a la lista.

La barra no se oculta con `set_visible(false)`: `GtkSearchBar` revela su contenido desde
`search-mode-enabled`, y esconder el widget lo sacaba del layout y el toggle no dibujaba nada.

## Paquetes

La barra lateral lista los paquetes con su nombre y cuántas descargas agrupan. Sobre el
seleccionado: **Pausar**, **Reanudar** y **Eliminar**. Eliminar un paquete **nunca** borra
archivos (`delete_file: false`), y las acciones colectivas omiten las tareas cuyo estado no
admite la operación.

Límite: no filtra la lista de descargas por paquete.

## Debrid al agregar

Si hay proveedor configurado y con token, el diálogo de alta resuelve el enlace antes de
encolar. Si el proveedor falla o devuelve algo que no es `https`, se descarga la URL original
sin mostrar error.

## VPN en el encabezado

Una píldora muestra el estado real de la interfaz configurada:

| Estado | Texto | Color |
| --- | --- | --- |
| `active` | `VPN activa (wg0)` | verde |
| `down` | `VPN caída (wg0)` | advertencia |
| `missing` | `VPN ausente (wg0)` | advertencia |
| `invalid` | `VPN mal configurada (wg0)` | error |
| sin configurar | *oculta* | — |

El estado lo calcula el **daemon** (`vpn.status`, con el mismo `ensure_up` que usa el kill
switch), no la ventana: si la ventana leyera el sistema por su cuenta podría mostrar “VPN
activa” mientras el daemon bloquea la descarga. Pulsar la píldora abre las preferencias.

## Control remoto

El diálogo muestra el estado del servidor, permite **Emparejar** y **Revocar**.

Al emparejar se muestra la URL de emparejamiento, el token **una sola vez** (con la advertencia
correspondiente) y un QR dibujado con el codificador Rust incluido: no hace falta `qrencode`.
Revocar invalida el token en el daemon y limpia la pantalla.

Los campos son seleccionables y el atajo de copiado del sistema alcanza; no hay botón propio.

## Suscripciones RSS

El panel lista nombre, URL, destino y último sondeo, y ofrece **Comprobar ahora**, **Quitar** y
un formulario de alta con nombre, URL, filtro y destino.

El panel no descarga feeds: usa los métodos del daemon (`rss.feed.list`, `rss.feed.add`,
`rss.feed.remove`, `rss.check_now`), así que las reglas SSRF siguen en un solo lugar. Cuando una
comprobación falla, se muestra cuántos feeds no se pudieron leer y el código del error en vez
de una lista vacía.

Límites: no se edita una suscripción (hay que quitarla y recrearla), el estado de error no se
guarda en la base —sobrevive solo a la comprobación que lo produjo— y la URL se valida en el
daemon, no en la ventana.
