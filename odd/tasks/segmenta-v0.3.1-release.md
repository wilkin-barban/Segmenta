# Segmenta v0.3.1 — preparación y publicación

Release en `main`; no afirmar verificaciones ni publicación sin evidencia. La Fase 1 empieza después de la publicación solicitada.

## Specs

- **S1.** “Subí la versión a 0.3.1 en Cargo.toml, en los manifests de extensiones (extension/chromium/manifest.json y extension/firefox/manifest.json) y agregá la release 0.3.1 en packaging/io.github.wilkin.Segmenta.metainfo.xml con fecha de hoy.”
- **S2.** “Redactá packaging/release-notes-0.3.1.md y actualizá CHANGELOG.md y README.md documentando los 6 fixes de estabilización de v0.3.0 (locale en CLI, saturación de disco, límite HTTP global compartido, SEGMENTA_SOCKET en daemon, error offline en extensiones, y reintento en capturas Xvfb), aclarando la deuda técnica de torrents/medios diferida para encarar en la Fase 1.”
- **S3.** “Compilá en release (cargo build --release --workspace), empaquetá las extensiones (node extension/tools/pack.mjs) y generá el paquete .deb final dist/segmenta_0.3.1-1_amd64.deb con cargo-deb.”
- **S4.** “Validalo con packaging/check-package.sh dist/segmenta_0.3.1-1_amd64.deb 0.3.1 y comprobalo instalado en este host local con sus smoke tests habituales.”
- **S5.** “Hacé el commit atómico en main sin Co-Authored-By ni menciones de IA ('release: prepare v0.3.1 with stabilization fixes and host validation').”
- **S6.** “Hacé git push origin main, creá el tag anotado v0.3.1 ('git tag -a v0.3.1 -m "Segmenta 0.3.1"'), pusheá el tag (git push origin v0.3.1) y publicá la release en GitHub con:
   gh release create v0.3.1 dist/segmenta_0.3.1-1_amd64.deb dist/segmenta-extension-0.3.1.zip dist/segmenta-firefox-0.3.1.zip --title "Segmenta 0.3.1" --notes-file packaging/release-notes-0.3.1.md”
- **S7.** “Asentá en el Log de odd/tasks/segmenta-v0.3.1-estabilizacion.md la publicación exitosa de v0.3.1 y el cierre formal de la Fase 0.”
- **S8.** “Abrí odd/tasks/segmenta-v0.4.0-automatizacion.md para dar inicio formal a la Fase 1 y comenzá con la tarea T1 (validar contratos IPC, capacidades torrent y postprocesado/inhibición).”

## Tasks

- [x] T1 — Actualizar versión y documentación de release (S1–S2) — ruta: manifests, metainfo, notas, README, CHANGELOG — commit: `c3beba4`; validado en `job-50`.
- [x] T2 — Construir y validar artefactos, verificar instalación y smoke en host (S3–S4) — ruta: `dist/`, packaging — commit: `c3beba4`; `job-50`, `job-51`, `job-52`.
- [x] T3 — Commitear, revisar y publicar en orden, sin inferir éxito (S5–S6) — ruta: Git, GitHub — commit: `c3beba4`; tag `v0.3.1` y release verificados.
- [x] T4 — Registrar publicación y comenzar investigación T1 de Fase 1 (S7–S8) — ruta: `odd/tasks/` — commit: pendiente; T1 de Fase 1 queda abierta.

## Log

- L1 — Pedido del usuario (verbatim):

> Che, el Product Owner dio la orden formal de cerrar la Fase 0, publicar la release v0.3.1 y arrancar la Fase 1.
>
> Necesito que encares los siguientes pasos en orden:
>
> 1. Subí la versión a 0.3.1 en Cargo.toml, en los manifests de extensiones (extension/chromium/manifest.json y extension/firefox/manifest.json) y agregá la release 0.3.1 en packaging/io.github.wilkin.Segmenta.metainfo.xml con fecha de hoy.
> 2. Redactá packaging/release-notes-0.3.1.md y actualizá CHANGELOG.md y README.md documentando los 6 fixes de estabilización de v0.3.0 (locale en CLI, saturación de disco, límite HTTP global compartido, SEGMENTA_SOCKET en daemon, error offline en extensiones, y reintento en capturas Xvfb), aclarando la deuda técnica de torrents/medios diferida para encarar en la Fase 1.
> 3. Compilá en release (cargo build --release --workspace), empaquetá las extensiones (node extension/tools/pack.mjs) y generá el paquete .deb final dist/segmenta_0.3.1-1_amd64.deb con cargo-deb.
> 4. Validalo con packaging/check-package.sh dist/segmenta_0.3.1-1_amd64.deb 0.3.1 y comprobalo instalado en este host local con sus smoke tests habituales.
> 5. Hacé el commit atómico en main sin Co-Authored-By ni menciones de IA ('release: prepare v0.3.1 with stabilization fixes and host validation').
> 6. Ya configuré la autenticación de GitHub en este host con gh. Hacé git push origin main, creá el tag anotado v0.3.1 ('git tag -a v0.3.1 -m "Segmenta 0.3.1"'), pusheá el tag (git push origin v0.3.1) y publicá la release en GitHub con:
>    gh release create v0.3.1 dist/segmenta_0.3.1-1_amd64.deb dist/segmenta-extension-0.3.1.zip dist/segmenta-firefox-0.3.1.zip --title "Segmenta 0.3.1" --notes-file packaging/release-notes-0.3.1.md
> 7. Asentá en el Log de odd/tasks/segmenta-v0.3.1-estabilizacion.md la publicación exitosa de v0.3.1 y el cierre formal de la Fase 0.
> 8. Abrí odd/tasks/segmenta-v0.4.0-automatizacion.md para dar inicio formal a la Fase 1 y comenzá con la tarea T1 (validar contratos IPC, capacidades torrent y postprocesado/inhibición).
>
> Metele tranquilo que te voy supervisando; yo sigo el ciclo de control.

- L2 — Confirmación posterior (verbatim):

> Che, impecable el plan en segmenta-v0.3.1-release.md. Tenés luz verde total: ejecutá de un tirón T1 (actualizar versión y documentación de release), T2 (construir binarios release, empaquetar .deb y extensiones, y validar instalación en este host), T3 (hacer el commit atómico, push a main, tag v0.3.1 y publicar con gh release create) y T4 (cerrar formalmente la estabilización y abrir Fase 1). Quedate trabajando hasta completar la publicación y el arranque de T1 de Fase 1.

- L3 — `job-50`: fmt, clippy, build, suite no GUI, Node 178, sync y suite GUI aislada pasaron. Paquete 0.3.1: gate 23/23, desktop y AppStream OK; lintian ausente. Metainfo y manifests 0.3.1 de ambos ZIP embebidos verificados. SHA-256 `.deb`: `936f8d5b2ad561962c4c8a1418e1644c956266b20dc7854d95a45c24c325493c`.
- L4 — `job-51`: instalado con respaldo SQLite íntegro y paquete anterior de recuperación validado; cinco binarios cotejados, versión, smoke GUI y capturas OK. `job-52`: daemon 0.3.1 y socket aislado OK, ambos ZIP instalados coinciden y cargaron en Chrome/Firefox con perfiles temporales. Procesos de la sesión previa no reiniciados; no se verificó visualmente el error de extensión.
- L5 — Commit `c3beba4`, push de `main` y tag anotado `v0.3.1` realizados; [release publicada](https://github.com/wilkin-barban/Segmenta/releases/tag/v0.3.1) y verificada por GitHub como no draft con los tres assets (`job-53` a `job-56`). Los digests remotos se reportaron `null`; no se afirma cotejo remoto. Revisión nativa `review-e22bacc809fe0a95` aprobada con aviso informativo sobre cobertura determinista de capturas.
