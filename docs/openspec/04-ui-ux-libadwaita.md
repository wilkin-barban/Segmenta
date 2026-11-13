# OpenSpec 04: Especificación de Interfaz de Usuario (GTK4 / Libadwaita)

> **Especificación:** OPENSPEC-SEGMENTA-004  
> **Versión:** 1.2.0  
> **Entorno Objetivo:** Ubuntu 26.04 LTS / GNOME 50+ / Wayland  
> **Toolkit:** GTK4 (>= 4.14) + Libadwaita (>= 1.5)

> **Normativo:** el contrato IPC y el ciclo de vida están en 07; el empaquetado, los execution
> budgets y las pruebas están en 10. La UI consume el daemon, no abre sockets ni descarga por su
> cuenta.

> **Alcance de esta revisión:** la GUI en el árbol es **0.1.2** (`crates/segmenta-gui`, versión del
> workspace `0.1.2`). Todo lo que aquí se describe **sin etiqueta existe hoy** en ese código; lo que
> pertenece a **0.2.0** se marca con **«(0.2, en preparación)»** y **no se puede leer como
> entregado**.

> **Cambio de contrato: no hay categorías en la UI.** El panel lateral no ofrece filtros por
> categoría y las Preferencias no ofrecen mapeo por categoría, subcarpetas automáticas ni
> clasificación de ningún tipo. El ajuste legacy `category_dirs` **se sigue leyendo** para no romper
> bases de datos antiguas, y nada en la UI lo edita ni lo usa para elegir destino. La fila de sólo
> lectura que la GUI 0.1.2 todavía pinta con ese mapeo es residuo, no contrato: se retira en U1
> (§8).

---

## 1. Filosofía de Diseño y Experiencia de Usuario

Segmenta sigue las **GNOME Human Interface Guidelines (HIG)** sobre GTK4 + Libadwaita:
* **Color de acento del sistema:** el widget de segmentos pide `WidgetExt::color()` a un widget con
  la clase CSS `.accent` (acento de Libadwaita) y sólo cae a un color propio si el tema todavía no
  lo define.
* **Soporte nativo de Wayland:** cero renderizado XWayland y escalado fraccionario sin artefactos.
* **Transición claro/oscuro:** la app sigue a `AdwStyleManager`; no impone un tema propio.
* **Guardado inmediato:** en Preferencias cada control guarda con `settings.update` (sin botón
  «Guardar») y muestra el rechazo del daemon como `AdwToast`.

---

## 2. Anatomía de la Ventana Principal

```
+-------------------------------------------------------------------------+
| [ + Añadir URL ]                              [ 🔍 ] [ ⚙ ] [ ☰ ] [ X ] |
+-------------------------------------------------------------------------+
|      Panel Lateral            |           Lista de Descargas            |
| ───────────────────────────── | ─────────────────────────────────────── |
| 📥 Todas (14)                 | 📦 ubuntu-26.04-desktop.iso             |
| ⚡ Activas (2)                 |    45.2 MB/s ─ 1.2 GB de 4.0 GB ─ 1 min |
| ⏸ Pausadas (1)                |    [▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░]   |
| ✅ Completadas (11)            |    [ ⏸ ] [ ✕ ]                          |
| ⚠ Fallidas (0)                | ─────────────────────────────────────── |
|                               | 🎬 documental_naturaleza_4k.mp4         |
| Colas                         |    12.8 MB/s ─ 850 MB de 1.8 GB ─ 2 min |
|   main                        |    [▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░]   |
+-------------------------------+-----------------------------------------+
```

### Componentes Estructurales
1. **`AdwHeaderBar`** (`build_layout`, `window.rs`):
   * `+ Añadir URL` como acción principal sugerida (`suggested-action`), atajo `Ctrl+N`.
   * `GtkToggleButton` de búsqueda (`system-search-symbolic`, atajo `Ctrl+F`) enlazado en dos
     sentidos con el `GtkSearchBar` inferior.
   * Botón de Preferencias (`emblem-system-symbolic`, atajo `Ctrl+,`).
   * `GtkMenuButton` (`open-menu-symbolic`) con el menú: Añadir URL, Recargar, Buscar,
     Preferencias, Acerca de, Salir (`Ctrl+Q`); «Acerca de» abre un `AdwAboutDialog` aparte.
   * **No hay** botones globales de «Reanudar todas» ni «Pausar todas».
2. **`AdwNavigationSplitView`**: panel lateral colapsable, ancho mínimo 210 px, fracción 0,24,
   máximo 320 px.
   * **Filtros** (`Filter::ALL`): Todas, Activas, Pausadas, Completadas, Fallidas, cada uno con
     icono, etiqueta localizada y contador; el contador y el filtro se calculan sobre el snapshot
     cargado, no en el daemon.
   * **Colas**: una fila por `queue_name` **presente en el snapshot de `download.list`**, con su
     número de tareas; seleccionar una filtra la lista. El grupo se oculta si el snapshot no tiene
     ninguna cola. La GUI no llama a `queue.list` ni permite crear, borrar o programar colas: eso
     hoy es CLI (`queue.create`, `queue.remove`, `queue.schedule`).
   * **Sin categorías**: el panel no tiene sección ni filtro de categorías.
3. **Pila de contenido (`GtkStack`)**: cuatro páginas distintas — `loading`, `list`, `empty`,
   `error` — para que «daemon inalcanzable» nunca se confunda con «no hay descargas». La página
   `loading` se desprende de la pila al llegar el primer estado listo para que el spinner detenido
   no quede pintado detrás de la lista.
4. **Lista de tareas (`GtkListView` con reciclaje de celdas)**: una sola petición
   `download.list {"filter":"all","limit":500,"offset":0}` por refresco; el filtro de la barra
   lateral y la búsqueda se aplican en cliente (`set_single_click_activate(false)`, la activación
   es explícita). Tope real: 500 tareas, sin paginación todavía.
5. **Página de detalle** (`AdwNavigationPage` con `tag = "detail"`): nombre, ruta final, estado en
   píldora, barra de progreso con porcentaje, velocidad, ETA, tamaño, mensaje de error, widget de
   segmentos con etiqueta accesible y acciones **Pausar**, **Reanudar**, **Cancelar** (con
   confirmación), **Eliminar** (con confirmación) y **Abrir carpeta**. Cada botón se muestra sólo
   si el estado lo admite: `pause` en `probing|downloading`; `resume` en `paused|failed`; `cancel`
   en `pending|probing|downloading|paused`; `delete` en `completed|cancelled|failed`;
   `open_folder` cuando hay ruta final.

---

## 3. El Widget Clave: Visualizador de Segmentos en Vivo (`SegmentaSegmentView`)

### 3.1. Arquitectura del Widget Personalizado
* Subclase de `gtk::Widget` (`segment_view.rs`) con `measure` y `snapshot`.
* La geometría vive en una función pura y testeable sin display, `layout_segments(segments, total_bytes)`.
* Pintado con `gtk::Snapshot::append_color` sobre `graphene::Rect`.

### 3.2. Geometría de Renderizado
Para un ancho total del widget $W$ y un tamaño total de archivo $S$:

1. Cada bloque $i$ ocupa en fracciones del ancho según su rango
   $[\text{start\_offset}_i, \text{end\_offset}_i]$; la suma de los rangos se usa como denominador
   cuando $S$ todavía es desconocido (división segura, sin NaN).
2. La porción descargada de dicho bloque es la fracción
   $(\text{next\_offset}_i - \text{start\_offset}_i)/(\text{end\_offset}_i - \text{start\_offset}_i)$.
3. Renderizado real: una pista (`track`) con el acento al 18 % de alfa y la parte descargada con el
   acento sólido. La separación entre bloques es el 6 % del ancho de bloque, acotada a 0,5–3 px, y
   sólo existe cuando hay más de un bloque.
   * **No** hay gradientes ni animación de pulso: la barra es estática y se repinta con cada evento.

---

## 4. Diálogos Especializados

### 4.1. Diálogo de Nueva Descarga (`add_dialog.rs`)
* `AdwDialog` con `AdwHeaderBar` (Cancelar / Iniciar) y un `AdwPreferencesGroup`.
* **Campos reales:**
  * URL de origen (`AdwEntryRow`).
  * Nombre de archivo (`AdwEntryRow`); se autocompleta con el basename de la URL hasta que el
    usuario escribe el suyo.
  * Carpeta destino (`AdwActionRow` que abre `GtkFileDialog` del portal); arranca con el
    `download_dir` del snapshot de ajustes.
  * Conexiones concurrentes (`AdwSpinRow`, 1–32; arranca con `max_connections` del snapshot y su
    respaldo es 16).
  * Interruptor «Comenzar ahora» (activo por defecto).
  * Etiqueta de error en línea, con `Enter` sobre la URL como atajo de envío.
* **Envío**: `download.add` con `queue_name: "main"` y `priority: 0` fijos; el botón se desactiva
  mientras se espera y, si el daemon no contesta en 60 s, el error se pinta en línea y el botón se
  reactiva.
* **Portapapeles:** si contiene una URL http(s) **y el campo URL está vacío**, se usa para
  prellenar el diálogo. Hoy no consulta `clip_monitor_enabled` (§8, deuda de U1).
* **Sin categorías ni subcarpetas**: el destino es una carpeta única.

### 4.2. Selección de calidad de medios
* La selección de calidad **no ocurre en la GUI**: vive en el panel flotante de la extensión
  (`extension/shared/panel.js`), con la escalera 4K/1080p/720p/360p + «Solo audio» y default 720p
  (03 §3.3).
* La GUI sólo consume el resultado: la tarea aparece en la lista y el detalle muestra su progreso,
  igual que cualquier otra descarga.
* **(0.2, en preparación)** no hay ningún diálogo GTK de calidades en el plan; si se añadiera, debe
  reutilizar la misma escalera y no inventar opciones.

### 4.3. Confirmaciones destructivas
* Cancelar y Eliminar usan un `AdwAlertDialog` con título y cuerpo localizados
  (`ConfirmCancelTitle`/`ConfirmCancelBodyLong`, `ConfirmDeleteTitle`/`ConfirmDeleteBodyLong`), la
  respuesta «Volver» como predeterminada y de cierre, y una sola respuesta destructiva.
* `download.cancel` y `download.delete` se envían con `delete_file: false` desde la UI: ningún
  botón borra el archivo del disco.

---

## 5. Integración con el Sistema Operativo Ubuntu

* **Continuidad en segundo plano:** no depende de un portal. El daemon es un servicio de usuario
  independiente, así que cerrar la ventana de la GUI no interrumpe ninguna descarga; al reabrir, la
  GUI se reconecta al socket y recarga con `download.list`.
* **Notificaciones de escritorio (`gio::Notification`):** al completarse o fallar una tarea se envía
  una notificación con el icono de la app y el nombre del archivo como cuerpo. Los títulos están
  **incrustados en español** («Descarga completada», «Descarga fallida», `window.rs`) aunque el
  catálogo ya tiene `Msg::CompletedNotification` y `Msg::FailedNotification` sin usar: es deuda de
  i18n, no contrato.
  * **No hay botones de acción** en la notificación («Abrir archivo» / «Mostrar en carpeta» no
    existen); abrir la carpeta es una acción del detalle.
* **Bandeja del sistema:** no implementada. GNOME no garantiza AppIndicator sin extensión, así que la
  app no asume `libayatana-appindicator3` ni promete un icono de bandeja.
* **Portal `org.freedesktop.portal.Background`:** no se usa. La instalación como servicio de usuario
  (`systemd --user`) es la que sostiene el daemon; el `.desktop` sólo lanza la GUI.
* **Modos de QA del binario** (`main.rs`): `--smoke` construye la ventana con una descarga sintética
  y sale al reflejarla, `--show-task ID` abre el detalle de una tarea y `--screenshots DIR` escribe
  las capturas claro/oscuro de 1440×900.

---

## 6. Ventana de Preferencias (`AdwPreferencesDialog`)

Se abre como `AdwPreferencesDialog` (640×720, búsqueda habilitada). Las páginas se añaden en este
orden y **estas son las que existen hoy** en `crates/segmenta-gui/src/prefs.rs`:

| Página (clave i18n) | Grupos | Controles reales |
| --- | --- | --- |
| **General** (`PrefsGeneral`) | **Carpetas** (`PrefsFolders`) y **Sesión** (título literal «Sesión», `prefs.rs:80`) | Fila «Carpeta de descargas» activable que abre el selector de carpeta del portal y guarda `download_dir`; interruptor «Iniciar con la sesión» (`autostart`). La fila de sólo lectura con el mapeo legacy de categorías que 0.1.2 todavía pinta no forma parte de este contrato: se retira en U1 (§8). |
| **Conexiones y límites** (`PrefsConnectionsSection`) | **Transferencia** (`PrefsTransfer`, descripción `RetriesHint`) y **Proxy** | `AdwSpinRow` de conexiones por descarga (1–32); límite global de velocidad en KiB/s (0–1 000 000, paso 128, `0` = sin límite); reintentos por tarea (0–50); filas **informativas** de modo (`system`/`none`/`manual`) y servidor (`—` si no hay); interruptor «Permitir direcciones locales» (`allow_local_downloads`). `proxy_mode`/`proxy_url` no se editan aquí. |
| **Descargas web** (`PrefsCapture`) | **Descargas web**, descripción `CaptureEnabledHint` | Interruptores «Capturar descargas del navegador» (`capture_enabled`), «Capturar archivos .torrent» (`capture_torrents`) y «Capturar enlaces magnet» (`capture_magnet`); fila «Extensiones capturadas» (recuento) con botón **Editar** que abre un editor de texto y guarda `capture_extensions`; fila «Nunca capturar desde» con el mismo editor para `domain_denylist`. |
| **Navegadores** (`PrefsBrowsers`) | **Navegadores** (descripción `BrowserInstallHint`) y **Alternativa local** (`ExtensionLocalTitle`) | Fila «Instalar manifiesto nativo» con botón → `extension.install-native` (toast con instalados/errores y refresco de estados). Una fila por navegador **instalado** según `extension.status` (se omiten los que no traen `command`): nombre, estado real — `native-ready` («Manifiesto nativo listo · Extensión no instalada»), `extension-installed`, `native-missing`, `unsupported` —, versión en sufijo atenuado, botón **Preparar extensión** (`extension.prepare` con `family`), botón **Abrir página de extensiones** (`extension.open` con `browser_id`) y **Copiar ruta** sólo cuando el navegador trae `prepared_path`. Segunda fila informativa con la ruta de las extensiones locales (`/usr/share/segmenta/extensions`). |
| **Acerca de** (`About`) | Grupo sin título | Fila «Segmenta» con `Versión <CARGO_PKG_VERSION>`; «Licencia» `GPL-3.0-or-later`; «Sitio web» activable que abre el repositorio; «Protocolo» (`ProtocolValue`); «Daemon», que se rellena con `system.status` como `versión · N s activo · M conexiones · secretos: <backend>` sin bloquear el diálogo. |

Páginas que **no existen todavía** y se documentan sólo como objetivo:

* **Contraseñas de sitios** — **(0.2, en preparación)**: página con añadir/editar/borrar por dominio,
  contraseña en diálogo aparte, usuario en SQLite y secreto en GNOME Keyring. Sólo HTTP Basic; sin
  Digest. La contraseña nunca se muestra ni se devuelve por `settings.get`. Contrato: 07 y 12;
  tarea L1 + U1.
* **Antivirus / post-proceso** — **(0.2, en preparación)**: interruptor de escaneo, motor detectado
  (`clamdscan`/`clamscan`), timeout, carpeta de cuarentena y el resultado por tarea
  (`quarantine_path`, códigos `ERR_ANTIVIRUS_*`). Hoy `postprocess_command` se **persiste pero el
  daemon no lo ejecuta** y no hay ningún control de antivirus en la UI. Contrato: 07, 08 y 12;
  tareas A1, A2 y U1.

Ajustes del daemon sin control en la UI (comportamiento actual, no contrato aspiracional):

* `capture_confirm` (default `true`): decide si la extensión abre la ventana de confirmación de
  captura. La GUI no lo expone; se cambia por CLI.
* `clip_monitor_enabled`: existe en el daemon y en el modelo de la GUI, pero el diálogo de añadir
  prellena el portapapeles sin consultarlo. Unificar semántica o retirar el ajuste es la tarea U1
  **(0.2, en preparación)**.
* `proxy_mode`/`proxy_url`: sólo lectura; se editan con `segmenta settings set`.
* `schedule_start`/`schedule_stop`, `action_on_complete`: sin control en la GUI; el tema es del
  scheduler (`queue.schedule`, 07 y 12), no de la UI (§8).
* `postprocess_command`: sin editor; la UI no muestra ni ejecuta el comando.

---

### 6.1. Gestión segura de descargas (0.2.1)

Dos acciones en el menú principal, ambas con confirmación cuando son destructivas:

* **Limpiar completadas**: habilitada sólo si hay filas `completed`. La confirmación nombra
  la cantidad y la acción borra únicamente el historial; **los archivos descargados no se
  tocan** y la interfaz no afirma lo contrario en ningún momento.
* **Eliminar** sobre una selección múltiple: el modo de selección se activa desde el menú y
  muestra una casilla por fila. La selección se guarda por identificador y sobrevive a un
  refresco que conserve las tareas.
* El diálogo de borrado dice cuántas tareas y cuántos archivos se verán afectados, avisa si
  la selección contiene tareas activas y ofrece la casilla **«Eliminar también los archivos
  descargados»**, desmarcada por defecto. No se cierra hasta conocer el resultado y después
  informa filas borradas, archivos borrados y cada fallo por nombre. Enter es la acción
  segura; Escape cierra.

## 7. Estados, accesibilidad y límites de la UI

Cada lista y cada diálogo define, como mínimo, estos estados:

- **Vacío:** `AdwStatusPage` con texto que explica cómo añadir la primera descarga y el botón de
  añadir; nunca una tabla vacía sin contexto.
- **Cargando:** página propia con spinner y etiqueta localizada; el hilo de GTK no se bloquea (el
  cliente IPC vive en un hilo trabajador y el resultado se aplica en un tick de 150 ms).
- **Error:** página propia con el mensaje del daemon (o el del cliente IPC), acción de reintento que
  descarta la conexión anterior y detalle seleccionable para copiar. La app no promete una
  comprobación de red que no hace: el texto pide revisar la unidad `systemd --user`.
- **Pausado/cancelado:** estados distintos con etiqueta propia; la pausa (y la tarea fallida) es
  reanudable y la cancelación no.
- **Daemon inalcanzable:** es el mismo estado de error; no hay una página aparte de «sin conexión».

### 7.1. Estados de captura (visibles en la extensión, no en la GUI)
| Estado | Dónde se ve | Significado |
| --- | --- | --- |
| Esperando confirmación | Ventana `confirm.html` | Descarga del navegador **pausada**; el usuario decide. |
| Aceptada | Panel/notificación de la extensión | La tarea existe en el daemon; la copia del navegador se cancela y borra **sólo** tras verificar bytes. |
| Rechazada | Ventana que se cierra | La descarga del navegador se reanuda; nada se envía. |
| Timeout (120 s) / ventana cerrada | Sin UI propia | El watchdog reanuda la descarga del navegador. |
| Copia conservada | Notificación de la extensión (`captureKeptTitle`/`captureKeptBody`) | La tarea nunca recibió datos: se reanuda el navegador, se descarta la tarea huérfana y el código interno es `ERR_CAPTURE_NOT_OWNED`. |
| En cuarentena | **(0.2, en preparación)** detalle de la tarea y Preferencias → Antivirus | Archivo movido fuera de Descargas, tarea `failed` con `ERR_ANTIVIRUS_FOUND` (A2, contrato 08/12). |

### 7.2. Estados de tarea mostrados por la GUI
La GUI conoce `pending`, `probing`, `downloading`, `paused`, `verifying`, `completed`, `failed`,
`cancelled` y `unknown`. Los estados que el daemon añada sin variante propia se pintan como
`unknown` sin romper la lista.

Consecuencia real de ese mapeo: `retrying` y `finalizing` existen en el daemon y en el esquema
(`store.rs`) pero la GUI los muestra como «Desconocida» y, como las acciones dependen del estado
conocido, una tarea en `retrying` no ofrece Pausar ni Cancelar desde la GUI. Es deuda declarada, no
un contrato: U1 debe darles etiqueta y acciones propias.

### 7.3. Estados de virus y cuarentena — **(0.2, en preparación)**
Nada de esto se renderiza hoy:

* `verifying` gana un subestado visible «Analizando» mientras corre el motor.
* Infección: la tarea pasa a `failed`, el detalle muestra «En cuarentena» con `quarantine_path` y el
  código `ERR_ANTIVIRUS_FOUND`; nunca queda el archivo en Descargas.
* Motor ausente con `antivirus_enabled`: tarea `failed` con `ERR_ANTIVIRUS_UNAVAILABLE`, sin
  publicar `completed`.
* Un fallo del hook o del motor no borra el archivo; se registra `postprocess_exit`/
  `postprocess_message` y la UI lo muestra como advertencia del detalle.

### 7.4. Requisitos transversales
- navegación completa por teclado y foco visible;
- etiquetas accesibles en iconos, barras de progreso y el widget de segmentos (la barra publica
  «Progreso: N %» y el visualizador publica su resumen de segmentos);
- contraste AA y respeto de `prefers-reduced-motion`;
- textos visibles desde el catálogo (`segmenta-i18n`: es, en, pt-BR).

---

## 8. Deuda declarada de la GUI 0.1.2 (insumo de U1)

Nada de esta lista es contrato: es lo que hay que corregir para que la UI cumpla el resto del
documento. Todas las tareas son **(0.2, en preparación)**.

| Deuda | Evidencia | Cierre |
| --- | --- | --- |
| Categorías visibles | `prefs.rs:58-77` pinta un `AdwExpanderRow` de sólo lectura con `category_dirs` | U1: retirar la fila; el setting se sigue leyendo para bases antiguas, sin UI y sin clasificar nada. |
| Cadenas incrustadas | Títulos de notificación (`window.rs:1188`, `window.rs:1215`), grupo «Sesión» (`prefs.rs:80`), filas «Modo»/«Servidor» (`prefs.rs:119`, `prefs.rs:124`), prefijo «Versión» (`prefs.rs:240`), tooltips `Buscar (Ctrl+F)`/`Preferencias (Ctrl+,)`, placeholder de búsqueda, textos de las páginas vacía/error, línea de `system.status` y botones del detalle (`Pausar`, `Reanudar`, `Cancelar`, `Eliminar`, `Abrir carpeta`) | U1: mover a `segmenta-i18n` (las claves `SessionSection`, `ProxyMode`, `ProxyServer`, `CompletedNotification` y `FailedNotification` ya existen) y añadir el gate que impida literales nuevos. |
| `clip_monitor_enabled` sin efecto | `add_dialog.rs` prellena sin consultar el ajuste | U1: una sola semántica, documentada, o retirar el ajuste de la UI. |
| Colas sin gestión | El panel deriva nombres de `download.list`; crear/borrar/programar es sólo CLI | U1: decidir si la UI expone `queue.*` o si el documento declara que la gestión de colas es CLI. |
| Tope de 500 tareas | `LIST_LIMIT` en `window.rs` | U1: paginación o límite documentado en 07. |
| `retrying`/`finalizing` desconocidos | `model.rs::DownloadStatus` sin esas variantes | U1: etiqueta y acciones propias (§7.2). |
| Site logins y antivirus ausentes | Sin RPC ni página | L1 + A2 + U1 (§6). |
