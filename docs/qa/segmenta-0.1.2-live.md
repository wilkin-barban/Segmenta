# Segmenta 0.1.2 — QA en vivo

Plan y registro de tareas: [`odd/tasks/segmenta-0.1.2.md`](../../odd/tasks/segmenta-0.1.2.md). Tag
`v0.1.2` en el repositorio. QA real ejecutado en Ubuntu el **2026-11-12**.

Este documento registra **sólo evidencia observada**: lo que corrió en Ubuntu se marca como
verificado, y lo que no se reprodujo queda como pendiente explícito en su propia sección.

## QA real en Ubuntu

| # | Verificación | Resultado |
| --- | --- | --- |
| 1 | Instalación del `.deb` 0.1.2 | **Verificado**: instalado como `0.1.2-1` |
| 2 | `extension.status` | **Verificado**: `native-ready` con `extension_installed: false` — manifiesto nativo listo y extensión no instalada, la distinción que pide la release |
| 3 | `extension.prepare` | **Verificado**: dejó las carpetas `chromium-0.1.2` y `firefox-0.1.2` |
| 4 | `extension.open` | **Verificado**: abrió Chrome |
| 5 | Panel en **Chrome for Testing** | **Verificado**: exactamente las opciones 4K/1080p/720p/360p/Solo audio, con 720p preseleccionado |
| 6 | «Descargar» desde el panel | **Verificado**: creó la tarea arriba de la lista y la tarjeta mostró «Enviado a Segmenta» |
| 7 | Captura `.zip` — aceptar | **Verificado**: pausó la descarga, mostró archivo, host y carpeta `/home/wilkin/Descargas`, y aceptar completó 524288/524288 B |
| 8 | Captura `.zip` — rechazar | **Verificado**: dejó el archivo al navegador sin crear tarea |
| 9 | Limpieza final | **Verificado**: audio y fixtures detenidos, `allow_local_downloads=false` |

## Gates en esta sesión

Ejecutados sobre el árbol de `v0.1.2` (no sustituyen el QA real; lo complementan).

| Gate | Comando | Resultado observado |
| --- | --- | --- |
| Rust | `cargo test --workspace` | **275 tests** en 46 suites, 0 fallos |
| Node | `node extension/tests/run.mjs` | **158 tests** verdes (incluye el watchdog de capturas) |
| Copias compartidas | `node extension/tools/sync-shared.mjs --check` | `ok: shared copies are in sync` |
| Empaquetado de la extensión | `node extension/tools/pack.mjs --check` | `ok chromium files=19` y `ok firefox files=19`, locales es/en/pt_BR |
| Paquete | `packaging/check-package.sh dist/segmenta_0.1.2-1_amd64.deb 0.1.2` | **22/22 rutas**; `appstreamcli validate: ok`; `lintian` no instalado en esta máquina (se omite, no falla) |

## Artefactos de la release

| Artefacto | SHA-256 |
| --- | --- |
| `dist/segmenta_0.1.2-1_amd64.deb` (7.139.400 B) | `f2b2aa3a78be4a66963f86df91f2e4c5be0b6f7713c6f760a477dea953b708a5` |
| `dist/segmenta-extension-0.1.2.zip` | `ac1e6453e322c61f10c64bfa33ab55f2a3f9db1f412b75b8d41705df7655938e` |
| `dist/segmenta-firefox-0.1.2.zip` | `110e252e47b3db4f62976a6eb09996cedbe46b431ca9a4a98868eb3e787d4855` |

## No reproducido en esta sesión

- **Firefox 0.1.2**: no se probó en esta ronda. El flujo de Firefox (carga temporal y
  `segmenta@wilkin.github.io`) sólo está cubierto por los tests de la extensión y por el QA de
  0.1.0/0.1.1; queda pendiente para el QA de 0.2.0.
- **Suspensión real del service worker MV3**: no se forzó la suspensión en el navegador. La
  rehidratación del watchdog está cubierta por los tests de extensión (`alarms` + registro
  persistido), no por una observación en el navegador.

## Limitaciones que siguen abiertas

- El primer download inmediatamente al arrancar el navegador puede ocurrir antes de que el
  service worker registre su listener; los downloads posteriores se capturan. R1 (0.2.0) agrega
  la reconciliación con `downloads.search` y el límite se documenta como irreducible.
- `torrent probe` de un `.torrent` multiarchivo todavía devuelve `total_bytes=0` y un único
  archivo de longitud 0; la descarga multiarchivo del motor sí funciona. R2 lo corrige.
- Firefox snap no puede ejecutar el host nativo; Firefox oficial funciona.
- El handler `magnet:` se instala como `.desktop` y puede requerir
  `xdg-mime default segmenta-handler.desktop x-scheme-handler/magnet` según el escritorio.
