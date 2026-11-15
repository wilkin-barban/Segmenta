# Segmenta 0.2.3 — tiers nominales de calidad en la extensión

## Specs

- **S1.** En `extension/shared/protocol.js` (y las copias `chromium/shared` y `firefox/shared`), agrupar por **tier nominal** de resolución, no por altura exacta: >=1800→2160/4K, >=900→1080, >=600→720, >=300→360, y el resto 240/144 si la UI los admite. Conservar la altura real y el `format_id` del formato elegido para que el request siga siendo válido.
- **S2.** `defaultQuality` elige el tier 720 por `quality`/tier, no por height exacto.
- **S3.** Test de regresión con las alturas reales `1184, 888, 592, 394, 296, 196, 128`: grupos 1080/720/360 y audio, `defaultQuality` 720. Además un caso 1920×888 / 1280×592.
- **S4.** Bump a **0.2.3** para extensión y paquete, ZIPs y `.deb` regenerados, instalado en Ubuntu y extensión preparada. Sin tag ni Release.
- **S5.** Pasos de reload y sha256 en el reporte. No tocar Cauce.

Causa confirmada por el usuario: `media.probe --kind video` de `FBhSudzU_ck` devuelve `1184, 888, 592, 394, 296, 196, 128` y `qualities` sólo audio; `groupQualities()` conservaba únicamente `2160/1080/720/360` exactos, no encontraba ningún video y caía al fallback de sólo audio. No es YouTube.

## Tasks

- [x] T1 — `qualityTier()` + `groupQualities()` por tier conservando altura real y `format_id` (S1) — `extension/shared/protocol.js` + copias sincronizadas — `c8bc18a`
- [x] T2 — `defaultQuality` por tier (S2) — mismo archivo — `c8bc18a`
- [x] T3 — Tests de regresión con las alturas reales y el caso 888/592 (S3) — `extension/tests/quality.test.mjs` — `c8bc18a`
- [ ] T4 — Bump 0.2.3, CHANGELOG y notas (S4)
- [ ] T5 — ZIPs y `.deb` regenerados, tests y `check-package.sh` (S4)
- [ ] T6 — `.deb` instalado y extensión 0.2.3 preparada (S4)
- [ ] T7 — Reporte con sha256 y pasos de reload (S5)

## Log

- L1 — Directiva del usuario (verbatim):

> El usuario sigue sin ver resoluciones con la extensión 0.2.2. Causa confirmada: para `FBhSudzU_ck`, `media.probe --kind video` devuelve formatos con alturas reales `1184, 888, 592, 394, 296, 196, 128` y `qualities` solo audio. `groupQualities()` conserva únicamente las alturas exactas `2160/1080/720/360`, así que no encuentra ningún video y cae al fallback de solo audio. No es YouTube.
>
> Corregí:
> 1. En `extension/chromium/shared/protocol.js` (y las copias shared/firefox), agrupá por **tier nominal** de resolución, no por altura exacta: p. ej. >=1800→2160/4K, >=900→1080, >=600→720, >=300→360, y el resto 240/144 si la UI los admite. Conservá la altura real y el `format_id` del formato elegido para que el request siga siendo válido.
> 2. Ajustá `defaultQuality` para elegir el tier 720 por `quality`/tier, no por height exacto.
> 3. Test de regresión con exactamente esas alturas: debe devolver grupos 1080/720/360 y audio, y `defaultQuality` 720. Agregá también un caso 1920×888/1280×592.
> 4. Bump a **0.2.3** para extensión/paquete, regenerá ZIPs y `.deb`, instalá en Ubuntu y prepará la extensión. Sin tag/release.
> 5. Dejá pasos de reload y sha256. No toques Cauce.
>
> TDD, commits ≤400, push a main. Si el contexto no alcanza, abrí una sesión nueva con esta directiva exacta; no simules capturas.
