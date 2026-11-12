# Segmenta 0.1.1 — QA en vivo

Plan: `/home/wilkin/.commandcode/plans/segmenta-0.1.1.md`. Release `v0.1.1`.

## Fases

| Fase | Estado | Evidencia |
| --- | --- | --- |
| T1 hover/back | **Verde** | `183b154`; `hovering_a_row_never_opens_the_detail`; GUI 11/11 |
| T2 settings, reglas y Preferencias | **Verde** | `642eabb`, `775bf1e`; página Descargas web; lista completa de 54 extensiones; reglas `r0*`/`r1*` |
| T3 detección e instalación | **Verde** | `7c32536`, `4b1061d`; página Navegadores; snap/Flatpak marcados; manifiestos por usuario; assets empaquetados |
| T4 captura del navegador | **Verde** | `76b45d0`, `bbb4b20`; `downloads.onCreated`, `contextMenus`, cancelación, `.torrent`/magnet; 76 tests Node |
| T5 torrents | **Verde** | `6c7879d`, `a33b2ff`, `6aa60d3`; schema v3, `librqbit`, motor compartido, RPC/CLI, cancelación y 46 suites |
| T6/T7 empaquetado | **Verde** | `.deb` 0.1.1, 22/22 rutas, `check-package`, clippy y gui smoke |

## Pruebas reales en Ubuntu

- Hover: no abre el detalle; clic normal sí. `segmenta-gui --smoke` → `SMOKE OK rows=1 segments=4`.
- `segmenta torrent probe` con magnet y `.torrent` HTTP.
- Motor torrent real: Sintel (`webtorrent.io/torrents/sintel.torrent`) descargó 91.160.439 B y se canceló correctamente.
- Captura Chrome for Testing: ZIP 524.288 B descargado desde HTTP local, cancelado en el navegador y completado por Segmenta.
- Firefox 160 oficial: extensión temporal, panel con calidades (prueba de 0.1.0; el manifiesto 0.1.1 conserva el mismo flujo).
- `yt-dlp 2026.10.26`, `ffmpeg 8.0.1`; `allow_local_downloads` se activó solo para el fixture HTTP local.

## Hallazgos abiertos

- El primer download inmediatamente al arrancar Chrome puede ocurrir antes de que el service worker registre el listener; los downloads posteriores se capturan. Documentado como limitación de arranque en frío.
- El motor reporta el total durante la descarga; el probe de `.torrent` multiarchivo aún devuelve `total_bytes=0` y un solo archivo de longitud 0.
- Firefox snap sigue sin poder ejecutar el host nativo; Firefox oficial funciona.
- El handler `magnet:` se instala como `.desktop`, pero el usuario debe registrarlo con `xdg-mime default segmenta-handler.desktop x-scheme-handler/magnet` si su escritorio no lo hace solo.

## Artefactos

- `segmenta_0.1.1-1_amd64.deb` — 6.8 MB — `a62123f795eb3855f8e4c676eb6f8395767d7f84d0fc5797f53a235c3ce7a450`
- `segmenta-extension-0.1.1.zip` — `342980b7cd33e1e5e9a48788385c2bc8705d619db4a59436eb81773ac6e08ee6`
- `segmenta-firefox-0.1.1.zip` — `95446187c127b0ec496ebf3e5f727816ebdafe22e36c2f1e0cba210f91040233`
- `segmenta_0.1.1-1_amd64.deb.sha256`
