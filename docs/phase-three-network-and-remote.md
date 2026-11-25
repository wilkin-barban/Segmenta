# Fase 3: red avanzada y control remoto

Estado: v0.6.0. Contrato y evidencia por tarea:
[`odd/tasks/segmenta-v0.6.0-red-remoto.md`](../odd/tasks/segmenta-v0.6.0-red-remoto.md).

## VPN con kill switch

Ajuste `vpn_interface`. Con un valor configurado, cada socket HTTP se ata al dispositivo
(`SO_BINDTODEVICE`, vía `HttpConfig::interface` → `ClientBuilder::interface`) y el daemon
comprueba la interfaz **antes** de crear el socket:

| Código | Significado |
| --- | --- |
| 1080 | `ERR_VPN_INTERFACE_INVALID` — nombre que el kernel no acepta |
| 1081 | `ERR_VPN_INTERFACE_MISSING` — la interfaz no existe |
| 1082 | `ERR_VPN_INTERFACE_DOWN` — existe pero está caída |
| 1083 | `ERR_VPN_TORRENT_UNSUPPORTED` — torrentes bajo VPN |

La idea es fail-closed: si el túnel no está, la descarga no sale por la ruta por defecto. Los
torrentes se rechazan porque librqbit abre sus propios sockets y no permiten atarlos: dejarlos
correr filtraría la IP real.

Requiere `CAP_NET_RAW` o root; sin ese permiso las conexiones fallan, que es el comportamiento
buscado. Con la interfaz arriba pero sin ruta al destino, la conexión se agota por timeout
(15 s por intento y los reintentos configurados), no de inmediato.

## Control remoto

Apagado por defecto (`remote_enabled`), puerto ≥1024 y bind limitado a `127.0.0.1` o
`0.0.0.0`.

```bash
segmenta remote pair --port 8765 --bind 0.0.0.0
segmenta remote status
segmenta remote revoke
```

`pair` acuña un token de 32 bytes, lo imprime una sola vez junto a la URL de emparejamiento y,
si `qrencode` está instalado, dibuja el QR en la terminal. En el almacén de secretos queda solo
el SHA-256.

API (toda ruta exige `Authorization: Bearer`):

- `GET /api/downloads` — hasta 50 tareas con estado, bytes descargados y total.
- `POST /api/action` — `{"task_id": "...", "action": "pause|resume|cancel"}`. Cancelar nunca
  borra el archivo.

`remote.revoke` borra el digest y cierra el puerto; el token anterior deja de servir de
inmediato. El transporte es HTTP sin TLS: es una superficie de LAN, no un reemplazo del socket
Unix, y no limita intentos de autenticación.

## RSS

Schema **v8** con dos tablas aditivas:

```sql
rss_feeds(id, url, name, filter_regex, destination, check_interval_mins, last_checked_at)
rss_seen_items(feed_id, guid, first_seen_at, PRIMARY KEY (feed_id, guid))
```

`ON DELETE CASCADE` limpia los GUIDs al borrar el feed. `rss_seen_items` es lo que hace que un
reinicio no vuelva a importar lo mismo.

IPC: `rss.feed.list`, `rss.feed.add` (url, nombre, filtro, destino, intervalo),
`rss.feed.remove`, `rss.check_now` (un feed o todos). El daemon además sondea cada 60 s los
feeds cuyo intervalo venció.

- **Filtro**: alternativas separadas por `|`, comparadas sin distinguir mayúsculas contra el
  título. Vacío importa todo; inutilizable no importa nada.
- **Parser**: RSS `<item>` y Atom `<entry>` con `href`; sin enlace `http(s)` el ítem se
  descarta y un documento truncado se rechaza entero.
- **SSRF**: esquema validado con `urlssrf`, y el host se resuelve y se rechaza si es una
  dirección bloqueada, salvo que `allow_local_downloads` esté activo.
- **Destino**: absoluto; el de la configuración cuando no se indica.

Los ítems que apuntan a un `.torrent` o a un magnet **no** se convierten en tareas de torrent
todavía.

## Suplantación TLS

No está incluida. `tls_profile` acepta `standard`; `chrome` se rechaza al guardar la
configuración con un error que nombra la feature faltante. El razonamiento completo está en
[`tls-impersonation-evaluation.md`](tls-impersonation-evaluation.md).
