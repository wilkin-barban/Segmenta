# Segmenta — ladder `qualities` por tier y errores de la extensión en Chrome

## Specs

- **S1.** Alinear el ladder `qualities` del daemon a **tiers nominales**, con los mismos cortes que la extensión: ≥1800 → 2160 (4K), ≥900 → 1080, ≥600 → 720, ≥300 → 360; por debajo de 300 no se ofrece. La etiqueta del tier no cambia (`4K`/`1080p`/`720p`/`360p`), el tier gana el mejor candidato y el ladder no debe contradecir a la extensión.
- **S2.** `media::default_quality` elige el tier 720 por tier, no por altura exacta, igual que `protocol.js`.
- **S3.** Remediar el error que la extensión reporta en Chrome en `shared/browser-api.js:176` (`i18n.getMessage`), con causa reproducida y test de regresión; nada de relajar validaciones ni capturar el error en silencio.
- **S4.** Verificar en el host `ssh ubuntu` (donde el usuario prueba la app) si es posible; si no, declarar el límite. Sin tag ni Release sin pedido explícito. No tocar Cauce.
- **S5.** TDD, commits ≤400 líneas.

Evidencia del usuario: en Chrome la extensión reporta un error con stack `shared/browser-api.js:176 (función anónima)`, que corresponde a `i18n.getMessage(key)`. En la línea 176 no hay ninguna otra operación.

## Tasks

- [ ] T1 — Ladder del daemon por tier + `default_quality` por tier, con tests (S1, S2)
- [ ] T2 — Diagnóstico y fix del error `browser-api.js:176` con test de regresión (S3)
- [ ] T3 — Verificación en el host ubuntu y reporte (S4)

## Log

- L1 — Pedido del usuario (verbatim):

> vamos ahora alinear el ladder qualities del daemon a tiers, ademas remedia los errores que me esta dando la extension en el navegador de chrome, de ser posible verifica justo en el host de Ubuntu que es donde estamos probando la aplicacion (ssh Ubuntu), te paso el error que me da justo en la extension en el navegador: shared/browser-api.js:176 (función anónima)
>
> [volcado completo de `extension/shared/browser-api.js`]

- L2 — Lectura del archivo: la línea 176 es `? i18n.getMessage(key)`. Un `key` que no sea string (p. ej. `undefined`) hace que Chrome lance de forma sincrónica «Invocation of form ... doesn't match definition», y el stack apunta a esa línea.
