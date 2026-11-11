# Segmenta 0.1.0 — tablero vivo

## Incidencia de empaquetado de la extensión (Chrome for Testing)

Chrome rechazó el ZIP con `Default locale was specified, but _locales subtree is
missing`. Causa: `manifest.json` declara `default_locale: es` y los catálogos viven en
`extension/_locales/`, fuera de `extension/chromium` y `extension/firefox`; el empaquetado
copiaba sólo el contenido del target.

Fix:

- `extension/tools/pack.mjs` copia `_locales` dentro de cada target (para que también
  funcione `--load-extension=extension/chromium`) y **valida antes de comprimir**:
  `default_locale` presente en `_locales`, los tres catálogos con el mismo conjunto de
  claves y cada `__MSG_*__` resuelto.
- ZIP propio (sin dependencias) con fecha fija, para que el asset sea reproducible.
- `extension/tests/pack.test.mjs` (6 tests) y encadenado en `extension/tests/run.mjs`.

Verificación: `unzip -l` muestra `_locales/{es,en,pt_BR}/messages.json` en ambos ZIP;
`node extension/tests/run.mjs` pasa con los 6 tests nuevos; el `.deb` no incluye assets
de la extensión, así que su SHA no cambia.
