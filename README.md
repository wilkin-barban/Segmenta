# Segmenta

> Gestor de descargas de alto rendimiento, concurrencia dinámica y captura de flujos multimedia para **Ubuntu (GNOME / Wayland)**, con partición en caliente de segmentos y cero tiempo de ensamblado en disco.

---

## 1. Visión y Propósito del Proyecto

**Segmenta** nace para llenar el vacío de un gestor de descargas aceleradas de grado comercial en Linux. Muchos usuarios de Ubuntu dependen de extensiones básicas o herramientas CLI (como `aria2` o `wget`), pero ninguna integra en un único flujo de trabajo:
1. **Segmentación dinámica en caliente (Dynamic File Splitting)**: Subdivisión de partes en tiempo real sin requerir una fase de ensamblado posterior.
2. **Captura profunda en el navegador (WebExtension Manifest V3)**: Detección inteligente de descargas, captura de cookies de sesión, headers HTTP (`User-Agent`, `Referer`, `Authorization`) e intercepción de flujos de streaming (HLS / MPEG-DASH).
3. **Experiencia de Usuario Nativa para Ubuntu 26.04**: Interfaz moderna basada en **GTK4 + Libadwaita**, respetando las Human Interface Guidelines (HIG) de GNOME, con soporte completo para Wayland, temas claro/oscuro del sistema y visualización de conexiones concurrentes en tiempo real.

---

## 2. Principios de Ingeniería

* **Diseño en Sala Limpia (*Clean-Room Design*)**: Proyecto desarrollado de forma 100% independiente a partir de especificaciones funcionales abiertas, estándares web (RFC 9110, WebExtensions MV3) y APIs nativas POSIX.
* **Cero Ensamblado (*Zero-Assembly Disk I/O*)**: Supera las limitaciones de los gestores tradicionales. En Linux, mediante `posix_fallocate()` y `pwrite64()`, todos los hilos de red escriben directamente en sus desplazamientos exactos dentro de un único archivo final.
* **Arquitectura Concurrente y Ligera**: Diseñado para el mínimo consumo de CPU y memoria (evitando frameworks pesados como Electron).
* **Documentación Llave en Mano**: Especificaciones abiertas (OpenSpec) detalladas paso a paso para que cualquier agente o desarrollador pueda implementar módulos de forma autónoma y determinista.

---

## 3. Estructura de Documentación del Repositorio

La documentación técnica y especificaciones completas se organizan en el directorio `docs/`:

```
Segmenta/
├── docs/
│   ├── research/
│   │   └── legacy-engine-analysis.md         # Análisis técnico de arquitecturas de descarga
│   └── openspec/
│       ├── 01-system-architecture.md         # Arquitectura del sistema, componentes e IPC
│       ├── 02-engine-dynamic-segmentation.md # Especificación del motor y algoritmo de partición
│       ├── 03-browser-extension-and-native-messaging.md # Extensión WebExtension y Native Host
│       ├── 04-ui-ux-libadwaita.md            # Especificación de la interfaz GTK4/Libadwaita
│       ├── 05-implementation-roadmap-for-agents.md # Guía paso a paso para agentes de desarrollo
│       ├── 06-http-contract.md               # Contrato HTTP, Accept-Encoding, ETag/If-Range y reintentos
│       ├── 07-ipc-contract.md                # Contrato JSON-RPC 2.0, SO_PEERCRED y eventos
│       ├── 08-persistence-recovery.md        # SQLite WAL, Safe Hot-Split y GNOME Keyring
│       ├── 09-extension-security.md          # Seguridad MV3, MessageChannel y Native Host en Rust
│       ├── 10-packaging-testing.md           # Cargo Workspace, empaquetado .deb y suite de tests
│       └── 11-decision-log.md                # Alcance v1, precedencia y definición de listo
├── LICENSE
└── README.md
```

---

## 4. Estado Actual

* [x] Investigación técnica y especificaciones de arquitectura completadas.
* [x] Especificaciones abiertas (OpenSpec) publicadas.
* [ ] Fase 1: Motor Central (`segmenta-core`).
* [ ] Fase 2: Daemon, IPC, CLI y Native Messaging Host.
* [ ] Fase 3: Extensión de Navegador.
* [ ] Fase 4: GUI GTK4 / Libadwaita.
* [ ] Fase 5: Sniffer y Muxer de Streaming (HLS / DASH).

---

## 5. Punto de entrada para desarrollar

1. Leer [`11-decision-log.md`](docs/openspec/11-decision-log.md): fija el alcance de v1 y la precedencia
   entre documentos.
2. Seguir [`05-implementation-roadmap-for-agents.md`](docs/openspec/05-implementation-roadmap-for-agents.md)
   para el orden de las fases.
3. Implementar HTTP con `06`, IPC con `07` y persistencia/hot-split con `08`.
4. Integrar la extensión/Native Messaging con `09` y empaquetar/probar con `10`.

La documentación describe una v1 ejecutable; el código y el andamiaje de Cargo se crean en la Fase 1.
La licencia del proyecto es GPL-3.0-or-later (ver [`LICENSE`](LICENSE)).
