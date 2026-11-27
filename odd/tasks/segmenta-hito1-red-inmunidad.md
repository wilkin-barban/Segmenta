# Segmenta Hito 1 — Red e Inmunidad

Hito siguiente al cierre del Hito 0 (`fd803a0` / `76abfb8`). Implementación sobre `main` con la
versión congelada en **0.7.0**; el tag y la release v0.8.0 se cortan recién al cerrar y probar
todo en el host.

Fuentes: [`docs/mdm-parity-architecture.md`](../docs/mdm-parity-architecture.md) (Parte I §1–§2,
Parte II Módulo A) y [`docs/tls-impersonation-evaluation.md`](../docs/tls-impersonation-evaluation.md).

## Investigación previa (evidencia del árbol, sin suposiciones)

### a) librqbit 9.1.0 — sí permite atar todo el tráfico

Leído en el crate vendorizado (`~/.cargo/registry/src/*/librqbit-9.1.0/src/`):

| Capacidad | Dónde | Qué cubre |
| --- | --- | --- |
| `SessionOptions.bind_device_name: Option<String>` | `session.rs:430-432` | Documentado como «What network device to bind to for **DHT, BT-UDP, BT-TCP, trackers and LSD**»; en Linux usa `SO_BINDTODEVICE` |
| `SessionOptions.dht: Option<DhtSessionConfig>` | `session.rs:427` | `None` desactiva DHT por completo |
| `SessionOptions.disable_local_service_discovery: bool` | `session.rs:480` | Corta el multicast de LSD |
| `SessionOptions.listen: Option<ListenerOptions>` | `session.rs:448` | Sin listener no hay puerto entrante; `ListenerOptions.enable_upnp_port_forwarding` (`listen.rs:55`) permite no pedir redirección al router |
| `SessionOptions.connect: Option<ConnectionOptions>` con `proxy_url` | `session.rs:450`, `session.rs:700-716` | Acepta SOCKS5 y también enruta los trackers HTTP por ese proxy |

Conclusión: **no hace falta fork**. La ruta es `bind_device_name = vpn_interface`, `dht` atado (o
`None`), LSD desactivado, UPnP apagado y, si se quiere, `connect.proxy_url` apuntando al relay
interno. El riesgo residual es que un socket creado antes de que la interfaz caiga siga escribiendo
en una ruta ya inválida: por eso se pausan las tareas ante la caída, no alcanza con el bind.

### b) rquest (BoringSSL) — no compilable en este host hoy

- `rquest` **no está** en `Cargo.lock` (0 coincidencias): sería una dependencia nueva y pesada.
- El host **no tiene** `cmake`, `clang`, `llvm-config` ni `libclang` instalados. Compilar BoringSSL
  exige la cadena C++ completa (CMake + Clang + libclang-dev) y varios minutos de build.
- Decisión de este hito: el **build base sigue sin C++**. La feature `impersonate` se deja como
  seam con su dependencia **opcional** y su validación de compilación se hace en un paso aparte, con
  autorización explícita para instalar toolchain. Mientras no esté compilada, `tls_profile: chrome`
  sigue rechazándose con error explícito; **nunca** se simula una huella que no existe.

## Specs

- **S1.** «Relay VPN interno en 127.0.0.1 (`crates/segmenta-daemon/src/vpn_relay.rs`): proxy loopback autenticado por sesión cuyos sockets salientes usen `SO_BINDTODEVICE` sobre `vpn_interface` y revaliden `ensure_up` (fail-closed). Inyectarlo limpio (`--proxy` y env vars) en `yt-dlp`, `ffmpeg`, `gallery-dl` y `rss`.»
- **S2.** «Torrents sobre VPN (`crates/segmenta-daemon/src/torrent.rs`): si librqbit 9.1.0 permite atar todos los sockets/tráfico (peers, trackers, listener) o enrutar por el relay desactivando UPnP y LPD sin fugar DHT/UDP, habilitarlo y pausar ante caída de VPN.»
- **S3.** «DNS cifrado (DoH / DoT) por la VPN: ajuste `dns_mode` (`system` | `doh` | `dot` + proveedores conocidos o custom) en `urlssrf`/probe y Preferencias, ejecutado por sockets atados a `vpn_interface` manteniendo las guardas anti-SSRF.»
- **S4.** «Suplantación TLS opcional (`--features impersonate` con rquest) + SiteRules y escalera anti-bot en `probe.rs` (tabla `site_rules` en SQLite/UI con `domain_pattern`, `user_agent`, headers en keyring, `max_connections` y flag browser-like, sin simular huellas falsas cuando la feature no está compilada).»
- **S5.** «Motor HTTP adaptativo (`segmenta-core/src/engine.rs`, `planner.rs`, `disk.rs`): `HostLimits` adaptativo con decaimiento temporal ante 429/503/cortes, evicción proactiva de workers lentos, tope de conexiones ampliable hasta 128 protegido por `HostLimits`/`SiteRules`, fallback sparse en disco, soporte PAC cuando aplique y acción `download.keep` para conservar streams HTTP sin `Content-Length`.»
- **S6.** «Mantenemos la versión en 0.7.0 durante la implementación en main (todavía NO cortar tag ni release v0.8.0 hasta cerrar y probar todo en el host). Todo directo a main, commits atómicos <=400 líneas con Conventional Commits y SIN Co-Authored-By ni atribución a IA. Todos los tests de los módulos nuevos bajo estricto aislamiento (nunca tocar `~/.local/share/segmenta` ni el keyring real) y gates completos al cierre de cada unidad (fmt, clippy --all-targets -D warnings, cargo test --workspace, xvfb-run -a cargo test -p segmenta-gui).»

## Tasks

- [x] T1 — Relay VPN interno: listener en `127.0.0.1` con autenticación por sesión, sockets salientes atados con `SO_BINDTODEVICE` y `ensure_up` revalidado **por conexión**; sin token válido no se atiende y sin interfaz arriba no se dial (S1) — ruta: `segmenta-daemon` — commit: `fcdd6d5` + `d0d1a4f`.
- [x] T2 — Inyección del relay en los procesos externos: `yt-dlp`, `ffmpeg`, `gallery-dl` y el sondeo de RSS salen por el relay cuando hay `vpn_interface`; se documenta qué herramienta respeta qué variable y qué se hace cuando no la respeta (S1) — ruta: daemon — commit: ver L5.
- [x] T3 — Torrents sobre VPN con `SessionOptions`: `bind_device_name`, DHT atado, LSD desactivado, UPnP apagado y listener opcional; pausa de las tareas al caer la interfaz; se retira `ERR_VPN_TORRENT_UNSUPPORTED` para el caso soportado y se conserva para lo que no se pueda garantizar (S2) — ruta: `torrent.rs`, `manager.rs` — commit: ver L7.
- [ ] T4 — DNS cifrado: `dns_mode` en ajustes con validación, resolución DoH/DoT por sockets atados a la interfaz, proveedores conocidos más uno custom, y las guardas anti-SSRF aplicadas al resultado (S3) — ruta: `segmenta-core`, daemon y Preferencias — commit: pendiente.
- [ ] T5 — SiteRules: tabla `site_rules` en SQLite con migración aditiva, `domain_pattern`, `user_agent`, `max_connections`, flag browser-like y headers en el almacén de secretos; aplicación en `probe.rs` con la escalera anti-bot (S4) — ruta: store, core y daemon — commit: pendiente.
- [ ] T6 — Feature `impersonate`: dependencia opcional `rquest`, `TlsProfile::Chrome` real solo en ese build, y comprobación de que el build base sigue sin C++ y sin rquest (S4) — ruta: `segmenta-core` — commit: pendiente.
- [ ] T7 — Motor HTTP adaptativo: `HostLimits` por host con decaimiento temporal, evicción de workers lentos, tope ampliable a 128 con guardas, fallback sparse, PAC y `download.keep` (S5) — ruta: `segmenta-core` y daemon — commit: pendiente.
- [ ] T8 — Preferencias y cierre: páginas para DoH y SiteRules, gates completos, prueba en host y notas de v0.8.0 **sin** cortar release todavía (S3, S4, S6) — ruta: GUI y documentación — commit: pendiente.

## Criterios de aceptación

- Con `vpn_interface` configurada, ningún proceso externo ni torrent abre un socket fuera de la interfaz: se demuestra con la interfaz arriba y con la interfaz caída, donde todo se detiene en vez de filtrar.
- El relay no atiende sin token de sesión y rechaza conexiones cuando la interfaz dejó de estar arriba, sin reintentos ciegos.
- `dns_mode` distinto de `system` resuelve por el túnel y sigue rechazando respuestas que apunten a direcciones bloqueadas por SSRF.
- `site_rules` se aplica por dominio sin aflojar SSRF ni el kill switch; un dominio sin regla se comporta como hoy.
- El build base no incorpora rquest ni BoringSSL, y `chrome` sigue fallando con un error que nombra la feature.
- `HostLimits` mejora el comportamiento ante 429/503 sin superar el tope por host, y `download.keep` conserva un stream sin `Content-Length` que hoy se descarta.
- Versión 0.7.0, gates completos en verde al cierre de cada unidad y árbol limpio.

## Pruebas previstas

- Relay: servidor HTTP local de prueba, token válido/inválido/ausente, interfaz inexistente y caída a mitad de conexión; verificación de que los procesos externos reciben la variable y de que el relay no resuelve el socket por defecto del usuario.
- Torrents: `SessionOptions` construida en una prueba unitaria (bind, LSD, UPnP, DHT), y comportamiento de pausa al caer la interfaz con una interfaz simulada.
- DNS: DoH contra un servidor local que devuelve respuestas válidas, NXDOMAIN y una IP privada (debe rechazarse), más el parseo de proveedores conocidos y custom.
- SiteRules: migración aditiva, coincidencia por patrón de dominio, aplicación de `max_connections` y de headers desde el secret store, y rechazo de patrones inválidos.
- Impersonate: el build por defecto **no** enlaza rquest; con la feature y el toolchain, `chrome` funciona (se documenta como paso separado).
- Motor: `HostLimits` con 429/503 y decaimiento simulado por reloj inyectable, evicción de un worker lento, tope 128, sparse en disco y `download.keep` con servidor de prueba sin `Content-Length`.
- Todas las suites nuevas con `SEGMENTA_SOCKET`/`XDG_*` temporales y `secret-tool` de mentira, como quedó en el Hito 0.

## Riesgos

- **Fuga de IP** si algún socket se crea antes del bind o sobrevive a la caída de la interfaz: es el riesgo principal del hito; se mitiga atando en el origen y pausando ante caída, y se verifica con la interfaz arriba y caída.
- **Rendimiento**: un relay en loopback agrega una copia por byte; hay que medirlo y permitir desactivarlo.
- **Compatibilidad de herramientas**: `yt-dlp`, `ffmpeg` y `gallery-dl` no usan las mismas variables (`--proxy`, `http_proxy`, `HTTPS_PROXY`); una que no respete el proxy filtraría. Se documenta por herramienta y se verifica cuál lo respeta.
- **BoringSSL**: sumar el toolchain (cmake, clang, libclang-dev) cambia el entorno de build y el tiempo de CI; se trata como decisión aparte.
- **Migración de datos**: `site_rules` es aditiva; `HostLimits` no toca la base.
- **Alcance**: el hito son cinco bloques; se entregan por unidades con gates propios para no acumular un cambio irrevisable.

## Log

- L1 — Pedido del usuario (verbatim, extracto íntegro de los cinco bloques y las reglas de entrega): arrancar el Hito 1 con sus cinco bloques —relay VPN interno, torrents sobre VPN, DNS cifrado, suplantación TLS opcional con SiteRules y motor HTTP adaptativo—; investigar antes de planificar qué soporta librqbit 9.1.0 (`bind_device`, listeners por IP, `socks_proxy_url`) y qué requiere rquest detrás de la feature `impersonate`; crear el ODD antes de tocar código; mantener 0.7.0 sin tag ni release; commits atómicos sin atribución a IA; tests herméticos y gates completos.
- L2 — Investigación (a): librqbit 9.1.0 **sí** permite atar todo el tráfico sin fork. Evidencia en `session.rs:430-432` (`bind_device_name` cubre DHT, BT-UDP, BT-TCP, trackers y LSD con `SO_BINDTODEVICE`), `session.rs:427` (DHT desactivable), `session.rs:480` (LSD desactivable), `listen.rs:52-60` (`ListenerOptions` con `enable_upnp_port_forwarding`, `listen_addr`, `ipv4_only`) y `session.rs:450` + `session.rs:700-716` (SOCKS5 en `connect.proxy_url`, también para trackers HTTP). No se detectó una fuga UDP insalvable, así que **no** corresponde levantar `ERR_VPN_TORRENT_UNSUPPORTED` como regla general: queda solo para configuraciones que no se puedan atar.
- L3 — Investigación (b): `rquest` no está en `Cargo.lock` y el host no tiene `cmake`, `clang`, `llvm-config` ni `libclang`. Compilar la feature `impersonate` exige instalar toolchain C++ (decisión aparte, con autorización). El build base se mantiene sin C++ y `chrome` sigue devolviendo el error que nombra la feature.
- L4 — Alcance de esta entrega: **solo planificación**. No se tocó código de aplicación; T1–T8 quedan pendientes de ejecución.

- L5 — T1 cerrado: relay con CONNECT y HTTP absoluto, `Proxy-Authorization` aceptando el token en usuario o contraseña, `ensure_up` antes de cada dial (`503`), SSRF sobre el destino con `allow_local` (`502` para loopback), y conexión no bloqueante (`EINPROGRESS` + espera de *writable*) con `copy_bidirectional`. Cuatro pruebas de integración: túnel autenticado, `407` sin token, `503` con interfaz inexistente y `502` al intentar pivotar a loopback. Commits `fcdd6d5` (relay) y `d0d1a4f` (pruebas).
- L6 — T2 cerrado: inyección del relay en los procesos externos. `vpn_relay::apply_env` / `apply_env_tokio` **eliminan** las variables de proxy heredadas (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` y sus variantes en minúscula, más `FTP_PROXY`) y publican la URL del relay. `run_process` (postproceso, antivirus, gallery-dl, extracción) aplica esa limpieza a **todo** binario externo; el sondeo de `yt-dlp` suma `--proxy` además del entorno; `gallery-dl` recibe `--proxy`; y el sondeo de RSS construye su cliente con `Proxy::all`. Sin relay activo el entorno queda intacto, así que un usuario sin VPN conserva su proxy del sistema.
  - Matriz documentada en las pruebas: `yt-dlp` respeta `--proxy` y las variables `*_PROXY`; `gallery-dl` respeta `--proxy`; `7z`/`unzip`/hooks heredan el entorno limpio aunque no hagan red; el sondeo RSS usa el cliente del daemon. **`ffmpeg` no se enruta**: en Segmenta solo se usa para concatenar archivos locales (HLS/DASH ya descargados), no abre red, así que no hay proxy que inyectar; queda escrito aquí para que no se asuma lo contrario.
  - Prueba unitaria en `vpn_relay` con un stub que imprime lo que recibe: el hijo ve `HTTP_PROXY`/`ALL_PROXY` apuntando al relay. La prueba de matriz en la suite confirma que sin relay no se toca nada y con relay `--proxy` aparece.
  - El relay arranca en `main.rs` solo si `vpn_interface` no está vacío; si el bind falla se registra y las herramientas siguen con su red propia (degradación explícita, no silenciosa).

- L7 — T3 cerrado: torrents sobre VPN. `torrent::session_options` construye `SessionOptions` con `bind_device_name = vpn_interface`, `disable_local_service_discovery = true` y `listen = None` (sin puerto entrante ni petición UPnP); DHT sigue activo pero atado al dispositivo. El daemon crea la sesión con `Session::new_with_opts`. Se retiró el rechazo global `1083`: el kill switch (`vpn_guard`) sigue negando el alta cuando la interfaz no existe o está caída (`1081`/`1082`), y el relay HTTP no interviene en torrents.
  - Riesgo residual tratado: `bind_device_name` cubre los sockets que librqbit crea, pero uno abierto antes de que el túnel caiga puede seguir vivo. El scheduler llama a `Manager::pause_torrents_if_vpn_down` en cada vuelta: si la interfaz dejó de estar arriba, pausa todas las tareas `torrent`/`magnet` en vuelo y lo registra. No se confía solo en el bind.
  - Pruebas: unitaria de las tres decisiones de `session_options` (bind, LSD, listener, y que sin VPN no se toca nada) y de integración que verifica `1081` al dar de alta un torrent con una interfaz inexistente. La prueba vieja del `1083` se reemplazó, no se eliminó.
  - Límite declarado: la sesión de librqbit es un `OnceCell` por proceso, así que cambiar `vpn_interface` exige reiniciar el daemon; queda anotado junto al ajuste.
