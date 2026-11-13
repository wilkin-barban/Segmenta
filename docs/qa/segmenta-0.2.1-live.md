# Segmenta 0.2.1 — QA live en Ubuntu

Fecha: 2026-11-13. Sin tag ni Release. Estado del `.deb`:
`57fa7a195c1ed9320a742f0412f5fff1bdb8d1f7bcb7fbb52f9457cd8fe00ffb`.

## Interfaz

- Instalado el `.deb` 0.2.1 y abierto con X11 en Ubuntu.
- El spinner central ya no aparece; la ventana muestra el cursor normal.
- Preferencias abre con las siete páginas: General, Conexiones y límites, Descargas web,
  Navegadores, Antivirus, Contraseñas de sitios y Acerca.
- Navegadores ya no muestra texto vertical; las descripciones envuelven y el scroll llega al
  final por encima de la barra inferior.
- El buscador negro del diálogo se retiró.

Capturas: `/tmp/seg-0215-main.png`, `/tmp/seg-0215-prefs.png`,
`/tmp/seg-0216-nav.png`, `/tmp/seg-0216-nav-bottom.png`.

## Gestión de descargas

Prueba física sobre la app instalada, con una descarga local de 4 KiB y otra de 8 KiB:

- **Limpiar completadas** con 1 tarea: contador a 0, toast «1 tarea(s) limpiada(s)», diálogo
  cerrado y archivo conservado.
- **Eliminar seleccionadas** con 2 tareas y casilla desmarcada: «se eliminaron 2 tarea(s); se
  borraron 0 archivo(s)».
- **Eliminar** 1 tarea con la casilla marcada: el `.zip` desapareció del disco y el toast dijo
  «se eliminaron 1 tarea(s); se borraron 1 archivo(s)».
- `allow_local_downloads` se restauró a `false`; los fixtures se eliminaron.

Capturas: `/tmp/seg-0216-clean-confirm.png`, `/tmp/seg-0216-cleaned.png`,
`/tmp/seg-0216-deleted.png`, `/tmp/seg-0216-deleted-files.png`.

## Cold-start Chromium

Tres arranques en frío con Chrome for Testing 157 y un perfil limpio. En los tres la descarga se
pausó, se abrió la ventana de confirmación y la tarea terminó en Segmenta; no quedaron descargas
pausadas ni tareas huérfanas.

Capturas: `docs/qa/screenshots/cold-start-confirm-run1.png` … `run3.png`.

## Firefox oficial

Firefox 160 con la extensión cargada temporalmente. El panel de YouTube mostró 4K, 1080p, 720p
(seleccionado), 360p y Solo audio; Native Messaging alcanzó `segmenta-host` y se creó la tarea.

Captura: `docs/qa/screenshots/firefox-youtube-panel.png`.

## Extensión en YouTube

Dos causas distintas, ambas corregidas:

1. **Código**: `validateSender` exigía `/watch` o `/playlist` en `sender.url`, pero tras una
   navegación SPA el content script puede reportar `/` o el origen. Producía `ERR_INVALID_SENDER`,
   traducido al texto genérico. `7dfda2c` acepta el origen exacto `https://www.youtube.com` y sigue
   validando el URL del payload. `944782a` cierra además el bypass de allowlist vacía. 168 tests
   Node verdes.
2. **Instalación**: el perfil real de Chrome tenía cargada una copia **0.1.1** desde
   `~/.local/share/segmenta/extension/chromium`, no la 0.2.x versionada. La carpeta se reemplazó por
   0.2.1 y la 0.1.1 quedó en `chromium-0.1.1-backup-20261113`.

Pendiente: pulsar **Recargar** en `chrome://extensions` y repetir el panel de YouTube en el Chrome
real. No se publica 0.2.1 hasta esa confirmación.
