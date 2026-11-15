# Segmenta 0.2.2 — detección de versión de extensión y bump de remediación

## Specs

- **S1.** `extension.status` debe exponer y comparar la versión del manifiesto preparado con la versión del paquete/archivo shipped; la GUI debe advertir si la extensión cargada está desincronizada, con acción para abrir `chrome://extensions`/recargar. Tests de los tres estados: coinciden, difieren, nada preparado.
- **S2.** Bump de extensión y paquete a **0.2.2**, CHANGELOG y notas con: soporte youtu.be/mix, menú contextual idempotente y detección de versión. No bumpees nada más.
- **S3.** Regenerá los ZIPs de extensión y el `.deb`; corré tests Node, cargo tests, fmt/clippy y `packaging/check-package.sh`. Dejá el `.deb` instalado en Ubuntu y la extensión `0.2.2` preparada.
- **S4.** No publiques tag ni GitHub Release de Segmenta sin pedido explícito. No toques Cauce.
- **S5.** Reportá archivos, tests, sha256 del `.deb`, ruta de la extensión y los pasos exactos para recargar en Chrome.

Contexto del bug original: Chrome mostraba `Cannot create item with duplicate id segmenta-add-torrent`; el backend ya detectaba el video (formats=7), el problema era extensión vieja/menú duplicado. TDD, commits ≤400, push a main.

## Tasks

- [ ] T1 — `extension.status` expone y compara versión preparada vs shipped (S1) — daemon `browsers.rs` + `manager.rs` — commit `feat(extension): compare prepared and shipped extension versions`
- [ ] T2 — GUI advierte desincronización con acción a `chrome://extensions`/recargar (S1) — `crates/segmenta-gui/src/prefs.rs`, `crates/segmenta-i18n/src/lib.rs` — commit `feat(gui): warn when the loaded extension is out of sync`
- [ ] T3 — Tests de los tres estados: coinciden, difieren, nada preparado (S1) — tests daemon + GUI — incluido en T1/T2
- [ ] T4 — Bump extensión y paquete a 0.2.2 + CHANGELOG + notas (S2) — commit `chore(release): bump to 0.2.2`
- [ ] T5 — Regenerar ZIPs y `.deb`, tests Node, cargo, fmt/clippy, `packaging/check-package.sh` (S3) — commit de artefactos si corresponde
- [ ] T6 — Instalar el `.deb` en Ubuntu y dejar la extensión 0.2.2 preparada (S3)
- [ ] T7 — Reporte final: archivos, tests, sha256, ruta de extensión, pasos de recarga (S5)

## Log

- L1 — Pedido del usuario (verbatim):

> Continuás el trabajo de Segmenta en una sesión nueva. Base: `main` en `daf7044`, pusheado, 170 tests Node verdes. Ya están hechos:
> - `17619ee`: soporte de enlaces cortos `youtu.be` y mixes.
> - `daf7044`: menú contextual `segmenta-add-torrent` idempotente (`removeAll`, error consumido, test de doble arranque).
>
> Falta cerrar la remediación:
> 1. `extension.status` debe exponer y comparar la versión del manifiesto preparado con la versión del paquete/archivo shipped; la GUI debe advertir si la extensión cargada está desincronizada, con acción para abrir `chrome://extensions`/recargar. Tests de los tres estados: coinciden, difieren, nada preparado.
> 2. Bump de extensión y paquete a **0.2.2**, CHANGELOG y notas con: soporte youtu.be/mix, menú contextual idempotente y detección de versión. No bumpees nada más.
> 3. Regenerá los ZIPs de extensión y el `.deb`; corré tests Node, cargo tests, fmt/clippy y `packaging/check-package.sh`. Dejá el `.deb` instalado en Ubuntu y la extensión `0.2.2` preparada.
> 4. No publiques tag ni GitHub Release de Segmenta sin pedido explícito. No toques Cauce.
> 5. Reportá archivos, tests, sha256 del `.deb`, ruta de la extensión y los pasos exactos para recargar en Chrome.
>
> Contexto del bug original: Chrome mostraba `Cannot create item with duplicate id segmenta-add-torrent`; el backend ya detectaba el video (formats=7), el problema era extensión vieja/menú duplicado. TDD, commits ≤400, push a main.
