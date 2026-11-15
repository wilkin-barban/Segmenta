# Segmenta 0.2.3 — tiers nominales de calidad en la extensión

## Specs

- **S1.** En `extension/shared/protocol.js` (y las copias `chromium/shared` y `firefox/shared`), agrupar por **tier nominal** de resolución, no por altura exacta: >=1800→2160/4K, >=900→1080, >=600→720, >=300→360, y el resto 240/144 si la UI los admite. Conservar la altura real y el `format_id` del formato elegido para que el request siga siendo válido.
- **S2.** `defaultQuality` elige el tier 720 por `quality`/tier, no por height exacto.
- **S3.** Test de regresión con las alturas reales `1184, 888, 592, 394, 296, 196, 128`: grupos 1080/720/360 y audio, `defaultQuality` 720. Además un caso 1920×888 / 1280×592.
- **S4.** Bump a **0.2.3** para extensión y paquete, ZIPs y `.deb` regenerados, instalado en Ubuntu y extensión preparada. Sin tag ni Release.
- **S5.** Pasos de reload y sha256 en el reporte. No tocar Cauce.
- **S6.** Verificado en vivo por el usuario en el host `ssh ubuntu` (el panel ya ofrece resoluciones y descarga el video). Documentar las mejoras acumuladas desde 0.2.1 —soporte `youtu.be`/mix, menú contextual idempotente, detección de versión y tiers nominales de calidad— en OpenSpec, README, CHANGELOG, notas de release y evidencia QA, y publicar `v0.2.3` con sus assets (`.deb`, `sha256`, ZIPs). Sin publicar `v0.2.2`: esa versión fue un paquete local de remediación, nunca distribuido.

Causa confirmada por el usuario: `media.probe --kind video` de `FBhSudzU_ck` devuelve `1184, 888, 592, 394, 296, 196, 128` y `qualities` sólo audio; `groupQualities()` conservaba únicamente `2160/1080/720/360` exactos, no encontraba ningún video y caía al fallback de sólo audio. No es YouTube.

## Tasks

- [x] T1 — `qualityTier()` + `groupQualities()` por tier conservando altura real y `format_id` (S1) — `extension/shared/protocol.js` + copias sincronizadas — `c8bc18a`
- [x] T2 — `defaultQuality` por tier (S2) — mismo archivo — `c8bc18a`
- [x] T3 — Tests de regresión con las alturas reales y el caso 888/592 (S3) — `extension/tests/quality.test.mjs` — `c8bc18a`
- [x] T4 — Bump 0.2.3, CHANGELOG y notas (S4) — `95f49b2`
- [x] T5 — ZIPs y `.deb` regenerados, tests y `check-package.sh` (S4) — `dist/segmenta-extension-0.2.3.zip`, `dist/segmenta-firefox-0.2.3.zip`, `dist/segmenta_0.2.3-1_amd64.deb`, gate `paths=23/23`
- [x] T6 — `.deb` instalado y extensión 0.2.3 preparada (S4) — `dpkg -s segmenta` = `0.2.3-1`, `daemon 0.2.3 (protocol v1)`, preparadas `chromium-0.2.3` y `firefox-0.2.3`
- [x] T7 — Reporte con sha256 y pasos de reload (S5)
- [x] T8 — Documentar las mejoras y publicar `v0.2.3` con assets (S6) — `fc2df81` + tag `v0.2.3` + Release con 5 assets

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

### Evidencia de cierre (2026-11-15)

- RED: con la suite en 170 verdes, los dos tests nuevos fallaron (`not ok 170/172 passed, 2 failed`); GREEN: `ok 172 tests`.
- Verificación sobre la extensión **preparada** en disco (`~/.local/share/segmenta/extensions/chromium-0.2.3/shared/protocol.js`): probe sintético con las alturas reales → `1080@1184, 720@888, 360@592, audio`, `defaultQuality` = `720`.
- `extension.status`: `package 0.2.3`, `chromium 0.2.3 / 0.2.3 / match`; manifiestos preparados en `0.2.3`.
- `packaging/check-package.sh dist/segmenta_0.2.3-1_amd64.deb 0.2.3` → `summary: OK … version=0.2.3-1 paths=23/23 lintian=not installed, skipped`.
- `.deb` sha256 `4c9726c88ab160671d9dffb61e8a3ffb3b83955546f9ef67f7404b117359d4a0`.
- `media.probe` real desde este host devolvió `1045 ERR_MEDIA_NOT_PUBLIC` para `FBhSudzU_ck` (IP de datacenter); la captura real la aportó el usuario y el test la reproduce byte a byte. No se simuló ninguna captura.
- Sin tag ni GitHub Release durante la implementación (S4); la publicación llegó después con autorización explícita del usuario. Cauce sin cambios.

### Publicación (2026-11-15)

- Docs: `fc2df81` — OpenSpec 03 §3.3 (tiers nominales) y 04 (panel + advertencia de versión en Navegadores), README (instalación y tabla de releases a 0.2.3, nota de que 0.2.2 no se publicó), CHANGELOG 0.2.3, `packaging/release-notes-0.2.3.md` reescrito para publicación y `docs/qa/segmenta-0.2.3-live.md` con la verificación del usuario.
- Tag anotado `v0.2.3` pusheado; GitHub Release publicada (no draft, no prerelease) con `segmenta_0.2.3-1_amd64.deb`, su `.sha256`, `segmenta_0.2.3-SHA256SUMS` (deb + ambos ZIP), `segmenta-extension-0.2.3.zip` y `segmenta-firefox-0.2.3.zip`.
- `v0.2.2` no se taggeó: nunca se distribuyó; sus cambios van incluidos en 0.2.3.
