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
| Keyring real, XDG y pause/resume honestos | Implementado, pendiente QA remoto | `134165e`; tests manager/secrets | Ninguno local |
| i18n es / pt-BR / en | Crate base commiteado; GUI/CLI en curso | `9b5f606`; `crates/segmenta-i18n` | Workers de localización fallaron; falta integrar en toda la UI |
| Arranque normal de la GUI | Corregido en código; pendiente verificación remota | `0beb18a`; `tests/startup.rs` | Falta probar contra daemon real en `ssh ubuntu` |
| Tema claro/oscuro y capturas | Capturas generadas y revisadas | `screenshots.rs` en curso; `docs/qa/screenshots/0.0.2-{light,dark}.png` 1440×900 | Falta commit de `screenshots.rs` y validación remota del `.deb` |
| Logging estructurado y redacción | Pendiente | Sin `logging.rs`; task T8 | Worker de logging falló; el agente principal debe retomarlo |
| Matriz de botones/acciones | Pendiente | Sin `tests/actions.rs`; task T12 | Depende de GUI/i18n |
| Empaquetado 0.0.2 | Pendiente | Versión local `0.0.1`; solo `.deb` 0.0.1 | Falta bump, build y `.deb` |
| QA remoto | Limpio en 0.0.1 | `ssh ubuntu`: sin tareas, `~/Descargas`, `allow_local_downloads=false` | Esperando candidato 0.0.2 |

## Reglas del flujo

1. Cada frente activo debe ser una **task visible** de `gentle-agents` con label
   `Segmenta · <frente>`, no un worker anónimo. Si falla, se reporta y se relanza con un
   alcance distinto; no se deja como tarea fantasma.
2. La sesión principal es el orquestador; las tareas hijas son las dueñas de cada edición.
3. No se publica `0.0.2` hasta instalar el `.deb` en `ssh ubuntu`, probar CLI **y GUI
   normal** (no solo `--smoke`), botones, keyring, XDG, pausa/reanudación, i18n, logs y
   capturas claro/oscuro.
4. Para capturas y temporales usar `mktemp -d` o una carpeta de scratch; evitar
   `rm -rf /tmp/...` que dispara confirmaciones de pérdida de datos. La limpieza final se
   hace en un paso autorizado y verificado.
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
