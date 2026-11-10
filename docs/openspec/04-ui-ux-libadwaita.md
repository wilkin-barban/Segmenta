# OpenSpec 04: Especificación de Interfaz de Usuario (GTK4 / Libadwaita)

> **Especificación:** OPENSPEC-SEGMENTA-004  
> **Versión:** 1.0.0  
> **Entorno Objetivo:** Ubuntu 26.10 / GNOME 48+ / Wayland  
> **Toolkit:** GTK4 (>= 4.14) + Libadwaita (>= 1.5)

---

## 1. Filosofía de Diseño y Experiencia de Usuario

Segmenta abandona el diseño tosco de ventanas legacy Win32 de los años 90 y abraza las directrices modernas de **GNOME Human Interface Guidelines (HIG)**:
* **Integración con Ubuntu Yaru:** Adaptación automática al color de acento del sistema (naranja de Ubuntu u opciones personalizadas).
* **Soporte Nativo de Wayland:** Cero renderizado XWayland, soporte para escalado fraccionario (125%, 150%) sin artefactos borrosos.
* **Transición Dinámica Claro/Oscuro:** Integración con `AdwStyleManager` para responder inmediatamente a los cambios de tema del escritorio.

---

## 2. Anatomía de la Ventana Principal

```
+-------------------------------------------------------------------------+
| [ + ] [ ▶ Todos ] [ ⏸ Pausar ]    Segmenta       [ 🔍 Buscar ] [ ⚙ ] [ X ]|
+-------------------------------------------------------------------------+
|      Panel Lateral      |                Lista de Descargas             |
| ─────────────────────── | ───────────────────────────────────────────── |
| 📥 Todas (14)           | 📦 ubuntu-26.10-desktop.iso                   |
| ⚡ Activas (2)           |    45.2 MB/s ─ 1.2 GB de 4.0 GB ─ Quedan 1 min|
| ⏸ Pausadas (1)          |    [▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░] |
| ✅ Completadas (11)      |    [ ⏸ ] [ ✕ ]                                |
|                         | ───────────────────────────────────────────── |
| Categorías:             | 🎬 documental_naturaleza_4k.mp4               |
| 📁 Documentos           |    12.8 MB/s ─ 850 MB de 1.8 GB ─ Quedan 2 min|
| 🎬 Vídeos               |    [▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░] |
| 🎵 Música               |    [ ⏸ ] [ ✕ ]                                |
| 📦 Comprimidos          |                                               |
+-------------------------+-----------------------------------------------+
```

### Componentes Estructurales
1. **`AdwHeaderBar`**:
   * Botón de acción principal: `+ Añadir URL` (sugiere automáticamente URLs válidas copiadas en el portapapeles).
   * Controles globales: Reanudar todas / Pausar todas.
   * Menú de hamburguesa: Opciones, Límite de velocidad global, Acerca de.
2. **`AdwNavigationSplitView`**:
   * Panel lateral colapsable automáticamente en pantallas pequeñas o modo mosaico en Ubuntu.
   * Filtros por estado y categorías inteligentes según el tipo MIME del archivo.
3. **Lista de Tareas (`GtkListView` con reciclaje de celdas)**:
   * Capaz de manejar miles de descargas históricas sin degradar los 60 FPS de la interfaz.

---

## 3. El Widget Clave: Visualizador de Segmentos en Vivo (`SegmentaSegmentView`)

Una de las características visuales más apreciadas por los usuarios es la barra gráfica que muestra cómo cada conexión avanza de forma independiente y cómo los bloques se dividen en caliente.

### 3.1. Arquitectura del Widget Personalizado
* Hereda directamente de `GtkWidget`.
* Dibuja los segmentos utilizando nodos de renderizado de GTK4 (`GskRenderNode` / `gtk_snapshot_append_color`).

### 3.2. Geometría de Renderizado
Para un ancho total del widget $W$ y un tamaño total de archivo $S$:

1. Cada segmento $i$ tiene un ancho proporcional en píxeles:
   $$\text{ancho}_i = W \times \frac{\text{end\_offset}_i - \text{start\_offset}_i}{S}$$
2. La porción descargada de dicho segmento ocupa:
   $$\text{descargado}_i = \text{ancho}_i \times \frac{\text{current\_offset}_i - \text{start\_offset}_i}{\text{end\_offset}_i - \text{start\_offset}_i}$$
3. Cada segmento se dibuja con un color diferenciado en tonos armónicos de la paleta de GNOME:
   * Segmento completado: Color de acento sólido.
   * Segmento en transferencia activa: Gradiente con animación sutil de pulso.
   * Porción no descargada: Fondo neutro atenuado (`@view_selected_hover_color`).

---

## 4. Diálogos Especializados

### 4.1. Diálogo de Nueva Descarga (`SegmentaAddDownloadDialog`)
* Basado en `AdwDialog` / `AdwPreferencesGroup`.
* **Detección Automática de Portapapeles:** Al abrirse el diálogo, si el portapapeles contiene un enlace `http://` o `https://`, se auto-rellena inmediatamente.
* **Campos:**
  * URL de origen (con botón de limpiar).
  * Selector de carpeta destino (con integración nativa con el portal XDG FileChooser).
  * Nombre de archivo (extraído del servidor o personalizable por el usuario).
  * Deslizador de conexiones concurrentes (de 1 a 32).
  * Interruptor (*switch*): "Comenzar descarga inmediatamente".

### 4.2. Diálogo Selector de Calidad de Video (Sniffer)
Cuando el sniffer del navegador detecta un video de YouTube, Vimeo u otra fuente con múltiples calidades:
* Muestra la miniatura del video, título y duración.
* Lista de variantes extraídas del manifiesto M3U8 o MPD:
  * `[ ○ ] 2160p (4K UHD) - 60 FPS - ~2.1 GB`
  * `[ ● ] 1080p (Full HD) - 60 FPS - ~450 MB (Recomendada)`
  * `[ ○ ] 720p (HD) - ~180 MB`
  * `[ ○ ] Solo Audio (M4A / 256 kbps) - ~25 MB`
* Botón: `Descargar Selección` o `Añadir a la Cola`.

---

## 5. Integración con el Sistema Operativo Ubuntu

* **Portal de Segundo Plano (`org.freedesktop.portal.Background`)**:
  * Permite que las descargas continúen activas aunque el usuario cierre la ventana principal.
  * Muestra el icono de estado en la bandeja del sistema mediante `libayatana-appindicator3` o el menú de estado de GNOME Shell.
* **Notificaciones de Escritorio (`org.freedesktop.Notifications`)**:
  * Al completarse una descarga, se emite una notificación nativa con dos botones de acción rápida:
    * `[ Abrir Archivo ]` -> Invoca a `xdg-open` sobre el archivo final.
    * `[ Mostrar en Carpeta ]` -> Abre el gestor de archivos de Ubuntu (Nautilus) con el archivo seleccionado.

---

## 6. Ventana de Preferencias y Opciones (`AdwPreferencesWindow`)

Para ofrecer una experiencia de configuración exhaustiva sin saturar visualmente al usuario, las opciones se estructuran en una ventana nativa de preferencias con pestañas laterales (`AdwPreferencesPage` y `AdwPreferencesGroup`):

### 6.1. Pestaña «General» (Integración y Teclas Modificadoras)
* **Inicio del Sistema:**
  * Interruptor (*switch*): `Iniciar Segmenta con la sesión de Ubuntu` (gestiona la unidad `systemd --user` o archivo `.desktop` en `~/.config/autostart`).
* **Monitoreo del Portapapeles:**
  * Interruptor: `Monitorear el portapapeles en busca de URLs de descarga válidas`.
* **Teclas de Anulación de Captura:**
  * Opción: `Tecla para omitir captura de Segmenta` (Predeterminado: Mantener pulsada la tecla `Alt` al hacer clic en un enlace permite que el navegador use su propio gestor nativo).
  * Opción: `Tecla para forzar captura inmediata` (Predeterminado: Mantener pulsada la tecla `Ctrl` fuerza la descarga con Segmenta aunque la extensión esté deshabilitada).
* **Integración del Sniffer de Navegador:**
  * Interruptor: `Mostrar panel flotante de descarga sobre videos web`.
  * Umbral de tamaño mínimo de video para mostrar el panel (ej. `Ignorar videos menores a 2 MB`).

### 6.2. Pestaña «Tipos de Archivos» (File Types & Exclusiones)
* **Extensiones a Capturar Automáticamente:**
  * Campo de texto con tokens editables:  
    `3GP 7Z AAC APK ARJ AVI BZ2 DEB DOC DOCX EXE FLAC GZ ISO M4A M4V MKV MOV MP3 MP4 OGG PDF RAR RPM TAR TGZ TORRENT WAV WEBM WMA WMV ZIP`
* **Lista Negra de Dominios (Exclusiones):**
  * Lista dinámica para añadir sitios web donde Segmenta **nunca** debe interceptar descargas (útil para intranets corporativas, portales bancarios o servicios web conflictivos).

### 6.3. Pestaña «Carpetas y Categorías» (Save To)
* **Mapeo de Rutas por Categoría:**
  * Cada categoría tiene asignada una carpeta de destino predeterminada con botón de exploración (`GtkFileDialog`):
    * 📁 **General:** `~/Descargas`
    * 📦 **Comprimidos:** `~/Descargas/Comprimidos`
    * 📄 **Documentos:** `~/Descargas/Documentos`
    * 🎵 **Música:** `~/Música/Segmenta` o `~/Descargas/Música`
    * 🎬 **Vídeos:** `~/Vídeos/Segmenta` o `~/Descargas/Vídeos`
    * ⚙️ **Programas:** `~/Descargas/Programas`
* **Comportamiento:**
  * Interruptor: `Recordar la última carpeta de guardado para cada categoría`.
  * Interruptor: `Crear subcarpetas mensuales automáticamente (ej. Descargas/2026-11/)`.

### 6.4. Pestaña «Conexión y Límites de Red»
* **Límites de Conexión:**
  * Desplegable: `Número máximo de conexiones por descarga` (Opciones: `1`, `2`, `4`, `8`, `16`, `24`, `32`). Predeterminado: `16`.
* **Límite Global de Velocidad (*Speed Limiter*):**
  * Interruptor: `Habilitar limitador de velocidad`.
  * Campo numérico con selector de unidad: `Velocidad máxima total` (ej. `5.0 MB/s` o `5120 KB/s`).
* **Excepciones de Conexión por Servidor:**
  * Tabla de reglas por dominio: Permite forzar a que servidores con protección anti-flood estricta (como Rapidgator o servidores universitarios) usen únicamente `1` o `2` conexiones concurrentes.

### 6.5. Pestaña «Programador y Colas» (Scheduler)
* **Gestión de Colas:**
  * Cola Principal (*Main Download Queue*) y opción de crear Colas Secundarias personalizadas.
  * Selector numérico: `Descargas activas simultáneas en la cola` (ej. `1` a la vez para máxima velocidad por archivo, o `3` en paralelo).
* **Programación Horaria:**
  * Interruptor: `Iniciar descarga de la cola automáticamente a las [ HH:MM ]`.
  * Interruptor: `Detener descarga de la cola a las [ HH:MM ]`.
* **Acciones al Completar la Cola:**
  * Menú desplegable:
    * `No hacer nada`
    * `Suspender el equipo`
    * `Apagar el equipo` (mediante `org.freedesktop.login1.Manager.PowerOff`)
    * `Cerrar Segmenta`

### 6.6. Pestaña «Contraseñas de Sitios y Proxy»
* **Contraseñas de Sitios (Site Logins):**
  * Gestor seguro de credenciales para sitios con autenticación previa (HTTP Basic, Digest, o FTP):
    * Servidor / Dominio (ej. `files.empresa.com`)
    * Usuario y Contraseña cifrados localmente en el gestor de claves del sistema (GNOME Keyring / `libsecret`).
* **Configuración de Proxy:**
  * Selector de modo:
    * `Usar configuración de proxy del sistema (GNOME)`
    * `Sin proxy (Conexión directa)`
    * `Configuración manual de proxy`:
      * Protocolo: `HTTP`, `HTTPS`, `SOCKS4`, `SOCKS5`
      * Host y Puerto
      * Autenticación (Usuario / Contraseña de proxy opcional)

### 6.7. Pestaña «Post-procesamiento y Antivirus»
* **Escaneo Automático de Seguridad:**
  * Interruptor: `Escanear archivos descargados al completarse`.
  * Campo de comando ejecutable: (ej. `/usr/bin/clamscan --bell -i %file%`).

