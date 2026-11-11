# Segmenta 0.1.1 — extensión instalable, captura web y torrents

Plan aprobado: `/home/wilkin/.commandcode/plans/segmenta-0.1.1.md`. Rama `main`, sin tags ni Release hasta el QA del usuario.

## Specs

- **S1.** «tiendas + fallback local»: la UI ofrece la tienda cuando exista listado y una ruta local guiada cuando no.
- **S2.** «captura en todos los sitios con denylist»: `downloads.onCreated` captura cualquier host salvo los de `domain_denylist`.
- **S3.** «motor embebido librqbit»: sin daemon externo; `magnet` y `.torrent`.
- **S4.** «seeding off por defecto».
- **S5.** «R0*/R1* como prefijos de extensión»: `r00`…`r99` cuentan como coincidencia por prefijo.
- **S6.** Hover nunca abre el detalle; un clic sí; «Atrás» lo cierra y limpia la selección.
- **S7.** Settings nuevos: `capture_enabled`, `capture_extensions` (lista completa del plan), `capture_torrents`, `capture_magnet`, con validación y deduplicación.
- **S8.** Detección de navegadores + `extension.status`/`install-native` + assets en `/usr/share/segmenta/extensions`; nunca `--load-extension` en Chrome estable ni escritura de perfiles.
- **S9.** Extensión: permisos `downloads`/`contextMenus`, cancelación de la descarga nativa, `download.add`, reglas cacheadas, `.torrent` → `torrent.add`, magnet por menú contextual.
- **S10.** Torrent: schema v3, `torrent.probe/add/status`, prefijo `torrent.` en el host, CLI y GUI; rutas hostiles rechazadas (`..`, absolutas, symlinks, escapes).
- **S11.** Empaquetado: extensiones desempaquetadas, `.desktop` `x-scheme-handler/magnet` + `application/x-bittorrent`, `check-package.sh` actualizado.

## Tasks

| ID | Spec | Entrega | Estado |
| --- | --- | --- | --- |
| T1 | S6 | `window.rs`: hover/clic/back + test de regresión GUI | hecho |
| T2 | S7 | Settings de captura, reglas puras y página Preferencias | pendiente |
| T3 | S8 | Detección de navegadores, RPC de extension, assets y página Navegadores | pendiente |
| T4 | S9 | Extensión: captura, torrent y tests Node | pendiente |
| T5 | S10 | Schema v3 + `librqbit` + RPC/CLI/GUI torrent | pendiente |
| T6 | S11 | Empaquetado, docs/QA y release notes 0.1.1 | pendiente |
| T7 | S1-S11 | Gates completos y `.deb` 0.1.1 | pendiente |

## Log

- L1: «Plan 0.1.1 aprobado. […] tiendas + fallback local, captura en todos los sitios con denylist, motor embebido librqbit, magnet + .torrent, seeding off por defecto; R0*/R1* como prefijos de extensión. […] No publiques nada.»
- L2: T1 implementado. `set_single_click_activate(false)`, guarda `click_intent` + foco en `selected_notify`, `connect_activate` para teclado, `click_task()` como gesto de clic y `select_task()` como selección sin activación. El test `hovering_a_row_never_opens_the_detail` fija la regresión; las suites GUI quedan verdes (11/11 en `actions`).
