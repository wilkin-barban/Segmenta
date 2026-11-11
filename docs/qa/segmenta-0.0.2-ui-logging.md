# QA visual y de integración — Segmenta 0.0.1 en Ubuntu 26.04

Fecha: 2026-11-11  
Host: `ssh ubuntu`, sesión GNOME 50/Wayland con `prefer-dark` y `Yaru-wartybrown-dark`.  
Capturas: `docs/qa/screenshots/segmenta-0.0.1-dark.png` y
`docs/qa/screenshots/segmenta-0.0.1-dark-theme-attempt.png`.

## Baseline observado

- La ventana abre a 970×680 sobre un escritorio 1440×900; no hay maximización inicial.
- El tema visual es oscuro y coherente con Yaru, pero **no se pudo validar el tema claro**:
  `GTK_THEME=Adwaita:light` no cambió la apariencia porque la sesión GNOME fuerza
  `color-scheme=prefer-dark`; hay que probar con `gsettings`/`AdwStyleManager` en una sesión aislada.
- Con una tarea completada en el daemon, la GUI quedó en **“Conectando con el daemon…”**
  después de 4 segundos y los contadores mostraron 0. `segmenta-gui --smoke` sí responde
  `SMOKE OK rows=1 segments=4`, así que el problema parece estar en el camino de conexión
  normal de la GUI o en su timeout/estado inicial, no en el motor.
- Todos los textos visibles están en español y codificados en la UI; no hay catálogos i18n.
- La ventana no muestra el daemon/versión ni un estado de reintento si la conexión falla después
  del mensaje inicial.
- El área central vacía ocupa la mayor parte de la ventana; el estado “Conectando…” no ofrece
  acción, comando ni detalle de diagnóstico.

## Pedidos para 0.0.2

### i18n

- Soportar `es`, `pt-BR` e `en`.
- Detectar el idioma del sistema, no una preferencia inventada:
  `LC_ALL`, `LC_MESSAGES`, `LANG`; normalizar `es_*`→`es`, `pt_BR`/`pt_BR.UTF-8`→`pt-BR`,
  `en_*`→`en`; fallback `en`.
- Traducir toda la GUI, estados, diálogos, errores, preferencias y textos accesibles; no dejar
  cadenas visibles hardcodeadas.
- Traducir también los mensajes humanos de la CLI y los errores del daemon que llegan a la UI.
- Tests de paridad de claves y de detección de locale (es, pt-BR, en, desconocido).
- No cambiar el protocolo JSON-RPC por i18n: los `code` estables siguen siendo la fuente; el texto
  se resuelve en el cliente.

### Tema claro/oscuro

- La app debe seguir el tema del sistema (`AdwStyleManager`/`color-scheme`) sin colores fijos
  que rompan contraste.
- Validar y capturar **light y dark** en Xvfb/GNOME aislado; no asumir que `GTK_THEME` alcanza.
- Revisar contraste, iconos simbólicos, foco visible, selección, barra de segmentos y estados de error
  en ambos temas.
- Añadir capturas 1440×900 para ambos temas al QA de release.

### Logging / auditoría

- Logging estructurado con `tracing`/`tracing-subscriber`: timestamp, level, target, `task_id`,
  `attempt_id`, action, state, HTTP status, error code, retry y duración.
- Salida a stderr/journald para el daemon (systemd ya captura en `journalctl --user -u segmenta`);
  opcional archivo JSONL con rotación y `0600` si se necesita persistencia.
- Niveles configurables por `RUST_LOG`/`SEGMENTA_LOG`; default `info`.
- **Nunca** loguear `Authorization`, cookies, cuerpos, credenciales, query strings con tokens ni
  secretos del keyring. Redactar la URL a scheme/host/path y descartar query/fragment.
- Loguear el ciclo de vida: arranque, migraciones, conexión IPC, add/probe, request/response,
  retries, pausa/resume, checkpoint, finalización, error, shutdown.
- La GUI debe loguear el fallo de conexión con el socket y el comando accionable, no sólo mostrar
  “Conectando…”.
- Añadir tests de redacción y de niveles; documentar cómo auditar con `journalctl`.

### Integración Ubuntu y pulido

- Si el socket no está activo, la GUI debe mostrar un estado accionable:
  `systemctl --user enable --now segmenta.socket`, con botón de reintento.
- Revisar el arranque normal de la GUI: `--smoke` funciona, `segmenta-gui` normal no llegó a listar
  la tarea completada en 4 s; diagnosticar con logs y corregir.
- Confirmar `.desktop`/icono/metainfo, notificaciones y arranque systemd; AppStream sin warnings.
- No agregar dependencias de UI pesadas; mantener el estilo GNOME/Yaru y la ventana usable a 970×680
  y 1440×900.

## Criterio de salida

La 0.0.2 no se publica hasta que:

1. La GUI normal (no sólo `--smoke`) conecte al daemon y liste una tarea completada en <2 s.
2. Existan capturas light/dark revisadas y el contraste sea correcto.
3. `LANG=es_ES`, `LANG=pt_BR.UTF-8` y `LANG=en_US.UTF-8` muestren la UI en es, pt-BR y en.
4. `journalctl --user -u segmenta` permita reconstruir un add, una pausa, una reanudación y un fallo
   sin secretos.
5. El remoto quede limpio y `segmenta list` vacío.
