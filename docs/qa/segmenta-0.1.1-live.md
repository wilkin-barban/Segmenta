# Segmenta 0.1.1 — QA en vivo (pendiente del usuario)

Plan: `/home/wilkin/.commandcode/plans/segmenta-0.1.1.md`. Nada publicado; `v0.1.0` intacto.

## Estado por fase

| Fase | Estado | Evidencia |
| --- | --- | --- |
| T1 hover/back | **Implementado y verificado** | `183b154`; `hovering_a_row_never_opens_the_detail`; suites GUI verdes |
| T2 settings + reglas + Preferencias | **Implementado y verificado** | `642eabb`, `775bf1e`; `segmenta-core::rules` (4 tests), validación de settings, página Descargas web |
| T3 detección de navegadores + RPC + assets | **Parcial** | detección con `snap`/`flatpak`, `extension.status`, `extension.install-native` (manifiestos por usuario, nunca sobrescribe), ZIPs y README en `/usr/share/segmenta/extensions`; falta la página **Navegadores** en la GUI |
| T4 captura del navegador | Pendiente | `downloads.onCreated`, `contextMenus`, `download.add`, cancelación nativa |
| T5 torrents (librqbit, schema v3) | Pendiente | `torrent.probe/add/status`, CLI, GUI, handler `magnet:` |
| T6/T7 empaquetado 0.1.1, docs, gates | Pendiente | bump de versión, notas de release, `.deb` 0.1.1 |

## Qué debe probar el QA (cuando T4/T5 estén)

1. Hover sobre la lista: el detalle **no** se abre; un clic sí; “Atrás” limpia la selección.
2. Preferencias → Descargas web: activar captura, editar extensiones (`r05` cae en `r0*`), denylist.
3. Navegadores: Chrome/Firefox oficiales “Listo”; snap reportado como no soportado; instalación del manifiesto por usuario.
4. Captura real: descarga `.zip`/`.mp4` desde el navegador → se cancela la nativa y aparece en Segmenta con su nombre.
5. `.torrent` multiarchivo y `magnet:` por menú contextual y por handler del sistema; rutas hostiles rechazadas.

## Riesgos abiertos

- `librqbit` aún no evaluado en este workspace: si su API no cubre magnet/DHT, se documenta antes de tocar el schema v3.
- La captura depende del permiso `downloads` (visible al instalar la extensión).
