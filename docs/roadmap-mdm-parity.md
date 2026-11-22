# Hoja de Ruta: Paridad Arquitectónica con Mister Download Manager (MDM)

El documento arquitectónico completo, estructurado punto por punto con especificaciones técnicas, rutas de código en el proyecto de referencia [anefzaoui/misterdownloadmanager](https://github.com/anefzaoui/misterdownloadmanager) y la guía de implementación para los crates de Segmenta, se encuentra en:

👉 **[docs/openspec/13-mdm-parity-and-advanced-roadmap.md](openspec/13-mdm-parity-and-advanced-roadmap.md)**

### Resumen de los 12 ejes de mejora analizados:
1. **Servidor MCP para Asistentes de IA:** Crate `segmenta-mcp` para control directo desde asistentes como Gentle Dot, Claude o Cursor.
2. **Cuentas y Servicios Debrid:** Desbloqueo nativo de hosts (Real-Debrid, AllDebrid, TorBox, Mega-Debrid).
3. **Click'n'Load 2 y Paquetes de Enlaces:** Servidor local en 127.0.0.1:9666 y agrupación de partes.
4. **Descompresión Automática Multiformato:** Extracción post-descarga de ZIP, TAR, 7Z y RAR con soporte de contraseñas.
5. **Galerías de Imágenes con `gallery-dl`:** Soporte para álbumes y redes sociales.
6. **Seguridad de Red: VPN Interface Binding y Kill Switch:** Descargas atadas a interfaces `tun0`/`wg0` con parada de emergencia.
7. **Suplantación TLS / Anti-Bot:** Evasión de bloqueos Cloudflare (BoringSSL/uTLS).
8. **Torrent Secuencial:** Streaming y previsualización de video en vivo durante la descarga.
9. **Canales RSS:** Automatización de descargas periódicas con filtros de texto.
10. **Extensiones Web Mejoradas:** Popup de descargas recientes en Chromium y Firefox.
11. **Control Remoto Web / Móvil:** Servidor web ligero emparejado por código QR para móviles o servidores NAS.
12. **Inhibición de Suspensión (D-Bus):** Prevenir la suspensión de la PC mientras haya transferencias activas.
