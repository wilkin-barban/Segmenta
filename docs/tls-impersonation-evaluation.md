# Suplantación TLS: evaluación y seam (T3 de la Fase 3)

Estado: **seam implementado**, suplantación real **no** incluida en el build base. Este
documento registra la evaluación que pide el módulo 7 de SPEC-013 y la decisión de
dependencias que todavía necesita autorización.

## Qué pide la especificación

`docs/openspec/13-mdm-parity-and-advanced-roadmap.md` §7 propone:

1. Evaluar `reqwest-impersonate` o `tokio-boring` como **feature opcional**
   (`--features impersonate`) para no penalizar el tiempo de compilación por defecto.
2. Añadir `impersonate_chrome: bool` en las opciones de sondeo, activado cuando el servidor
   responde con un desafío de Cloudflare o bloqueo de User-Agent.

## Evaluación

| Estrategia | Realidad hoy | Costo | Veredicto |
| --- | --- | --- | --- |
| `tokio-boring` / `boring` (bindings a BoringSSL) | Mantenido, pero compila BoringSSL en C++ con CMake y `clang`; añade minutos al build y una cadena de herramientas nueva | Alto | Solo detrás de feature, nunca por defecto |
| `reqwest-impersonate` | Proyecto renombrado y sin mantenimiento activo con ese nombre; la ruta viva es `rquest`, un fork de reqwest con `impersonate` sobre BoringSSL | Alto (misma cadena C++) | Preferible a `boring` directo si se autoriza |
| JA3/JA4 "a mano" sobre rustls | rustls no expone el orden de extensiones ni las curvas para replicar una huella de Chrome; saldría una huella distinta que también se bloquea | Medio, resultado incierto | Descartado |

Punto clave: **no existe suplantación barata**. Cualquier opción real arrastra BoringSSL y su
cadena de compilación, así que la única forma de cumplir "sin ralentizar el build base" es
compilarla como feature opcional y que el binario publicado no la incluya.

## Lo implementado en este paso

Sin dependencias nuevas, para que la decisión de sumar BoringSSL la tome el dueño del
proyecto y no un commit silencioso:

- `TlsProfile` (`standard` | `chrome`) en `segmenta-core::probe`, con `parse`/`name`.
- `HttpConfig.profile`, por defecto `standard`.
- `HttpConfig::unsupported_profile`: en un build sin la feature, pedir `chrome` devuelve un
  error explícito que nombra la feature faltante. **No hay fallback silencioso**: devolver
  la huella estándar haciéndose pasar por Chrome mantendría los 403 que la función existe
  para evitar, y encima mentiría sobre lo que hizo.
- `HttpConfig::interface_error` corre **antes** que el perfil en `Session::open`: el kill
  switch de T2 sigue teniendo prioridad sobre cualquier detalle de transporte, y ninguna de
  las dos comprobaciones puede saltear a la otra.
- Ajuste `tls_profile` en el daemon, validado en el límite de configuración: `chrome` se
  rechaza al guardar, no al descargar, para que nadie descubra el problema con una descarga
  fallando.

## Pruebas

RED observado antes de implementar (el campo y el tipo no existían: el test no compilaba) y
después en verde:

- `the_standard_profile_is_the_default`
- `asking_for_chrome_without_the_feature_is_an_explicit_error`
- `the_interface_is_checked_before_the_profile`
- `a_chrome_tls_profile_is_refused_by_a_build_without_impersonation` (integración contra el
  daemon real: `settings.update` con `chrome` devuelve el error que nombra la feature, un
  valor desconocido se rechaza y `standard` sigue funcionando)

## Lo que falta, y la decisión que se necesita

Para completar T3 hace falta autorizar **una** de estas dos rutas:

1. **Feature `impersonate` con `rquest`**: se agrega como dependencia opcional de
   `segmenta-core`, `Session::open` construye el cliente de `rquest` con el perfil Chrome en
   ese build, y el `.deb` publicado sigue sin la feature. Requiere CMake y BoringSSL en la
   máquina que compile esa variante, y actualizar `Cargo.lock` (ítem 5 de riesgo).
2. **Cerrar T3 como seam**: se documenta que Segmenta no suplanta huellas TLS y se retira la
   promesa del roadmap para v0.6.0.

Sin autorización no sumo BoringSSL: es una dependencia de cadena de compilación nueva,
pesada, y el propio módulo 7 la quiere opcional.

## Activación automática, todavía sin implementar

El punto 2 de §7 (activar el perfil al detectar un desafío de Cloudflare) necesita primero
una detección fiable. Sin la feature compilada no tiene sentido cablearla, así que queda
explícitamente pendiente y no se simula.
