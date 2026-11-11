# Segmenta 0.0.2 — tablero vivo

Actualizado: 2026-11-11 (supervisión cada 5 min)

Este tablero es la vista completa del frente. La sección de `gentle-shell`/Herdr muestra
solo tareas activas; las tareas fallidas o completadas no aparecen ahí. Para el historial
usar `gentle:agents` en la sesión de Segmenta; para ver este tablero:

```bash
watch -n 10 cat docs/qa/segmenta-0.0.2-live.md
```

## Frentes

| Frente | Estado | Evidencia / commit | Bloqueo |
| --- | --- | --- | --- |
| Keyring real, XDG y pause/resume honestos | Implementado, verde local | `134165e`; `tests/manager.rs`, `tests/secret_backend.rs` (15 tests) | Falta QA remoto |
| i18n es / pt-BR / en | **Parcial; falla en QA remoto** | `3c38151`; tabla traducida, pero estados `failed/completed` y `--help` siguen en inglés; ETA PT en español | Corregir estados, ayuda y ETA |
| Arranque normal de la GUI | **Falla en QA remoto (P1)** | `0beb18a`; conecta el socket (`active_connections=3`) pero queda en “Conectando…” y no lista tareas | Corregir el camino de conexión real; `--smoke` no alcanza |
| Tema claro/oscuro y capturas | Implementado; capturas revisadas | `49d32b7`; `--screenshots`, `docs/qa/screenshots/0.0.2-{light,dark}.png` 1440×900 | La GUI normal sigue bloqueada antes de mostrar datos |
| Logging estructurado y redacción | **Verificado en remoto con P2** | `9c1af8b`; journal reconstruye add/pause/resume/failed; sin secretos | `secret_backend` y `checksum_ok` salen `[redacted]` aunque no son secretos |
| Matriz de botones/acciones | 9 tests locales verdes | `3c38151`; `tests/actions.rs` | No se pudo probar en remoto porque la GUI no carga |
| Empaquetado 0.0.2 | Generado y validado; **no aprobado** | `dist/segmenta_0.0.2-1_amd64.deb` (3.8 MB); `check-package.sh`: 10/10 paths, AppStream y desktop OK | La QA remota rechazó la release |
| QA remoto | **Ejecutado: NO PUBLICAR** | `docs/qa/segmenta-0.0.2-e2e.md`; GUI normal atascada + i18n parcial | Corregir P1 y repetir el ciclo |

## Reglas del flujo

1. Cada frente activo debe ser una **task visible** de `gentle-agents` con label
   `Segmenta · <frente>`, no un worker anónimo. Si falla, se reporta y se relanza con un
   alcance distinto; no se deja como tarea fantasma.
2. La sesión principal es el orquestador; las tareas hijas son las dueñas de cada edición.
3. No se publica `0.0.2` hasta instalar el `.deb` en `ssh ubuntu`, probar CLI **y GUI
   normal** (no solo `--smoke`), botones, keyring, XDG, pausa/reanudación, i18n, logs y
   capturas claro/oscuro.
4. Para capturas y temporales usar `mktemp -d` o una carpeta de scratch privada; **no**
   usar `rm -rf /tmp/...` (dispara confirmaciones de pérdida de datos). La limpieza final
   se hace en un paso autorizado y verificado.
6. Las capturas 0.0.2 light/dark ya fueron revisadas por el usuario: claro y oscuro se ven
   correctos; no se vuelven a generar salvo que cambie la UI de forma visible.
5. Todo a `main`, sin ramas. El remoto se deja sin tareas, sin archivos de prueba y con
   `allow_local_downloads=false`.

## Criterio de cierre

- [ ] `segmenta-gui` normal lista una tarea completada en <2 s.
- [ ] Todos los controles de la matriz tienen handler y test.
- [ ] Capturas `0.0.2-light.png` y `0.0.2-dark.png` 1440×900 revisadas.
- [ ] i18n funciona con `LANG=es_ES`, `LANG=pt_BR.UTF-8`, `LANG=en_US.UTF-8`.
- [ ] `journalctl --user -u segmenta` reconstruye add/pause/resume/error sin secretos.
- [ ] `.deb` 0.0.2 + SHA-256 instalado y probado en `ssh ubuntu`.
- [ ] Tag `v0.0.2`, GitHub Release, `main` limpio y espacio liberado.

## Correcciones tras el QA remoto de 0.0.2 (2026-11-11)

| Bloqueo | Causa | Fix | Evidencia |
| --- | --- | --- | --- |
| GUI normal queda en «Conectando…» | El `activate` creaba el `SegmentaWindow` (envoltorio de `Rc<Inner>`) y lo descartaba: GTK mantenía el widget, pero moría el estado con el tick que procesa las respuestas. | `app::build_window_for_client` registra la ventana en `OPEN_WINDOWS` (thread-local). | Reproducido con el daemon real local: antes no había `finish reload`; ahora `download.list ok (3 rows)` y la lista se pinta. Test `the_production_window_is_kept_alive_by_the_application`. |
| i18n: estados crudos, `--help` en inglés, ETA en español | El CLI imprimía el protocolo tal cual y clap no localiza; la GUI componía «Quedan 53 s». | `render::status_label` traduce el estado; `--help` sale de `help_text(locale)`; `format_eta` devuelve sólo la duración y la etiqueta sale del catálogo. | `render_locale` (6 tests: estados es/pt-BR/en, help es/pt/en), test de modelo para la línea de progreso. |
| Redacción excesiva | `is_sensitive` hacía coincidencia por subcadena y «secret» atrapaba `secret_backend`; «checksum» ocultaba `checksum_ok`. | Coincidencia exacta o por sufijo (`_token`, `_secret`, `_password`, `_api_key`, `_cookie`). | Tests de `logging.rs`: `secret_backend=keyring` y `checksum_ok=true` visibles, `access_token` redactado. |
