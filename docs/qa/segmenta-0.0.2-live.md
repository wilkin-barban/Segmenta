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
| i18n es / pt-BR / en | Crate base verde; GUI/CLI sin integrar | `9b5f606`; `crates/segmenta-i18n` (4 tests) | Integración pendiente en GUI y CLI |
| Arranque normal de la GUI | Corregido y probado con socket real | `0beb18a`; `tests/startup.rs` (2 tests, <2 s) | Falta QA remoto |
| Tema claro/oscuro y capturas | Implementado y capturado | `49d32b7`; `--screenshots`, `docs/qa/screenshots/0.0.2-{light,dark}.png` 1440×900 revisadas | Falta QA remoto |
| Logging estructurado y redacción | En curso (agente principal) | task T8; sin `logging.rs` todavía | Ninguno |
| Matriz de botones/acciones | Pendiente | task T12; sin `tests/actions.rs` | Depende de i18n de GUI |
| Empaquetado 0.0.2 | Pendiente | versión local `0.0.1` | Falta bump, build y `.deb` |
| QA remoto | Limpio en 0.0.1 | `ssh ubuntu`: sin tareas, `~/Descargas`, `allow_local_downloads=false` | Esperando candidato 0.0.2 |

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
