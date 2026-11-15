# Segmenta — ladder `qualities` por tier y errores de la extensión en Chrome

## Specs

- **S1.** Alinear el ladder `qualities` del daemon a **tiers nominales**, con los mismos cortes que la extensión: ≥1800 → 2160 (4K), ≥900 → 1080, ≥600 → 720, ≥300 → 360; por debajo de 300 no se ofrece. La etiqueta del tier no cambia (`4K`/`1080p`/`720p`/`360p`), el tier gana el mejor candidato y el ladder no debe contradecir a la extensión.
- **S2.** `media::default_quality` elige el tier 720 por tier, no por altura exacta, igual que `protocol.js`.
- **S3.** Remediar el error que la extensión reporta en Chrome en `shared/browser-api.js:176` (`i18n.getMessage`), con causa reproducida y test de regresión; nada de relajar validaciones ni capturar el error en silencio.
- **S4.** Verificar en el host `ssh ubuntu` (donde el usuario prueba la app) si es posible; si no, declarar el límite. Sin tag ni Release sin pedido explícito. No tocar Cauce.
- **S5.** TDD, commits ≤400 líneas.

Evidencia del usuario: en Chrome la extensión reporta un error con stack `shared/browser-api.js:176 (función anónima)`, que corresponde a `i18n.getMessage(key)`. En la línea 176 no hay ninguna otra operación.

## Tasks

- [x] T1 — Ladder del daemon por tier + `default_quality` por tier, con tests (S1, S2) — `e658bc0`
- [x] T2 — Diagnóstico y fix del error `browser-api.js:176` con test de regresión (S3) — `e4d19da`
- [x] T3 — Verificación en el host ubuntu y reporte (S4) — `.deb` 0.2.4-1 instalado, probe real correcto

## Log

- L1 — Pedido del usuario (verbatim):

> vamos ahora alinear el ladder qualities del daemon a tiers, ademas remedia los errores que me esta dando la extension en el navegador de chrome, de ser posible verifica justo en el host de Ubuntu que es donde estamos probando la aplicacion (ssh Ubuntu), te paso el error que me da justo en la extension en el navegador: shared/browser-api.js:176 (función anónima)
>
> [volcado completo de `extension/shared/browser-api.js`]

- L2 — Lectura del archivo: la línea 176 es `? i18n.getMessage(key)`. Un `key` que no sea string (p. ej. `undefined`) hace que Chrome lance de forma sincrónica «Invocation of form ... doesn't match definition», y el stack apunta a esa línea.

### Evidencia de cierre (2026-11-15)

- **Causa del error de Chrome**: `daf7044` usó `API.getMessage('contextMenuAddTorrent')` sin agregar la clave a `_locales`. Chrome devuelve `''` y deja un `runtime.lastError` («Could not load the message») sin leer; la consola de la extensión lo atribuye a `shared/browser-api.js:176`, que es exactamente `? i18n.getMessage(key)` (verificado con `nl -ba`).
- **RED del daemon**: con la suite verde se agregó el test con las alturas reales y falló con `left: ["audio"]`, la misma salida que veía el usuario.
- **RED de locales**: el test ampliado (incluye `background.js` y los tres idiomas) falló con `locale key missing for contextMenuAddTorrent in en`.
- **GREEN**: `cargo test --workspace` 408 verdes; `node extension/tests/run.mjs` 172 verdes; `fmt` y `clippy -D warnings` limpios; `check-package` 23/23.
- **Verificación en vivo en `ssh ubuntu`** (0.2.4-1 instalado, daemon 0.2.4): `media probe` real de `FBhSudzU_ck` devuelve `formats` `1184, 888, 592, 394, 296, 196, 128` y un ladder `1080@1184, 720@888, 360@592, audio` con `default_quality` `720` — el ladder del daemon ya coincide con la extensión. La clave `contextMenuAddTorrent` está presente en `en`, `es` y `pt_BR` de la extensión preparada.
- `dist/segmenta_0.2.4-1_amd64.deb` sha256 `c9f9f8d958c25029e17b1e822b6af4d6a5551a5b1fcdec0993e841bfb0aedc1d`.
- Sin tag ni Release: pendiente de decisión explícita del usuario.
