# Segmenta 0.0.2 — QA remoto (Ubuntu 26.04, `ssh ubuntu`)

Fecha: 2026-11-11  
Paquete: `dist/segmenta_0.0.2-1_amd64.deb` (3.8 MB)  
Instalado: `0.0.2-1`; `segmenta 0.0.2`; `segmenta-gui 0.0.2`  
Host: Ubuntu 26.04.1 LTS, GNOME/Wayland, `XDG_RUNTIME_DIR=/run/user/1000`.

## Resultado

**No publicar.** La GUI normal no es usable y quedan fallos de i18n. La release no cumple el criterio de salida.

## P1 — La GUI normal no carga las tareas

Con el daemon activo y cuatro tareas en la base, `segmenta-gui` abre la ventana, muestra
`Conectando con el daemon…` y deja **todos los contadores en 0**. La captura
`docs/qa/screenshots/0.0.2-normal.png` se tomó a los 5 s; la segunda a los 12 s
(`0.0.2-normal2.png`) muestra lo mismo.

Evidencia:

- `segmenta --json list -f all` devuelve tareas completadas y fallidas.
- Mientras la GUI está abierta, `system.status` informa `active_connections=3`,
  `event_subscribers=1`; `ss -xap` muestra sockets establecidos de `segmenta-gui` y el
  socket `LISTEN` del daemon. No es un fallo de permisos ni de socket ausente.
- `segmenta-gui --smoke` y `--screenshots` funcionan, pero usan el cliente/demo y no
  prueban el camino de conexión real.
- `gui.log` está vacío; no hay diagnóstico de por qué el estado queda en `Loading`.
- La matriz local `tests/actions.rs` pasa 9/9, pero no cubre la conexión normal real.

Esto contradice S23 y el criterio de salida de 0.0.2.

## P1 — i18n de CLI incompleta

Con `LC_ALL=es_ES.UTF-8` y `LC_ALL=pt_BR.UTF-8`:

- Los encabezados de tabla sí se traducen (`ESTADO`, `TAMAÑO`, `VELOCIDADE`, etc.).
- Los **valores de estado siguen en inglés**: `failed`, `completed` en español y
  portugués.
- `segmenta --help` sigue completamente en inglés en ambos locales.
- La captura PT de `--screenshots` traduce “Baixando”, “Concluídas”, “Com falha” y
  “Adicionar URL”, pero el ETA sigue en español: “Quedan 53 s” (`0.0.2-pt-light.png`).
- No se pudo validar la GUI normal en pt/es porque antes queda en `Loading`.

## P2 — Redacción excesiva en logs

`journalctl --user -u segmenta` muestra eventos estructurados útiles y **no contiene**
`SUPERSECRET`, `token=`, `Bearer` ni cabeceras. Sin embargo:

- `secret_backend=[redacted]` oculta `keyring`, que no es secreto.
- `checksum_ok=[redacted]` oculta un booleano de auditoría.

La redacción de URL y cabeceras es correcta; conviene no redactar campos no sensibles.

## Verificado correctamente

- Instalación/upgrade con `apt`; `Depends` incluye `libsecret-tools`.
- `system.status`: `daemon_version=0.0.2`, `secret_backend=keyring`.
- `settings.download_dir=/home/wilkin/Descargas`; descarga HTTPS sin `-d` cayó ahí.
- Descarga segmentada de 32 MB con 8 conexiones; pausa → `speed_bps=0`, `eta_seconds=null`,
  estado `paused`; reanudación → `completed`; SHA-256 coincide.
- `pause`/`resume` sobre tarea fallida: exit 1, códigos `1011`/`1003`, estado sigue
  `failed`.
- Logs de ciclo de vida: add, engine start, pause, resume, completed, failed; IDs y
  duraciones; sin secretos.
- `segmenta-gui --version` y `segmenta --version` = 0.0.2.
- AppStream: válido (`redundante: 1`); `desktop-file-validate`: ok.
- Capturas 0.0.2 light/dark: 1440×900, contraste correcto; la captura PT está en
  `0.0.2-pt-light.png`.

## Segunda pasada — candidato `6bc4aad` (11:29)

Reinstalado `0.0.2-1` con el `.deb` regenerado (SHA `0d800a66…`). Corregidos y
reverificados:

- La GUI normal ya lista la tarea completada y los contadores en <5 s
  (`0.0.2-fixed-normal.png`); el socket real queda en `active_connections=1`.
- `segmenta list` en es/pt muestra `Completada`/`Concluído`; `--help` ya está traducido.
- La captura PT normal muestra `Concluído`, `Adicionar URL`, `Concluídas` y `Com fila`.
- Journal: `secret_backend="keyring"` y `checksum_ok=None` ya son visibles; no hay
  secretos en la URL/cabecera de prueba.

Quedan dos defectos P2 antes de publicar:

1. **Spinner residual:** la GUI normal deja un indicador giratorio en el centro del área
   vacía aunque el estado ya es `Ready` y la tarea está listada
   (`0.0.2-fixed-normal.png`, `0.0.2-normal-pt-fixed.png`). Debe ocultarse al terminar
   la carga.
2. **`--screenshots` no es determinista:** bajo `LANG=pt_BR.UTF-8` capturó
   `Conectando ao daemon…` y no mostró la tarea, aunque había una completada en el
   daemon (`0.0.2-pt-screenshot-failure.png`). El modo de capturas debe esperar el
   estado listo y no producir imágenes de carga; su test sólo verifica que los archivos
   existen y no vacíos.

No publicar hasta corregir ambos y repetir esta comprobación.

## Criterio para reintentar

1. La GUI normal debe listar las tareas del daemon en <2 s, no sólo `--smoke`.
2. Estados y `--help` deben respetar `es`/`pt-BR`/`en`; ETA y textos accesibles incluidos.
3. Dejar de redactar `secret_backend`/`checksum_ok` si no son sensibles.
4. El spinner debe desaparecer al llegar a `Ready`.
5. `--screenshots` debe capturar el estado listo, con una tarea visible, en ambos temas.
6. Reinstalar el siguiente `.deb`, repetir este informe y no publicar hasta que pase.

## Artefactos

- `docs/qa/screenshots/0.0.2-normal.png`
- `docs/qa/screenshots/0.0.2-normal2.png`
- `docs/qa/screenshots/0.0.2-pt-light.png`
- Paquete: `dist/segmenta_0.0.2-1_amd64.deb` + `.sha256`
