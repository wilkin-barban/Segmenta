# Segmenta 0.3.1

Release de estabilización de la base v0.3.0; no incorpora las funciones nuevas de la Fase 1.

## Correcciones

- **Locale de CLI:** el test de descarga reanudada fija el idioma de la consulta, evitando fallos cuando el host usa portugués.
- **Espacio libre:** la multiplicación amplía los operandos y conserva la saturación a `u64::MAX` sin conversiones redundantes en Clippy.
- **Límite global HTTP:** las transferencias HTTP comparten el mismo presupuesto; los cambios de tasa se aplican en caliente, incluidas las esperas activas.
- **Socket del daemon:** `SEGMENTA_SOCKET` selecciona el socket; `--socket` tiene precedencia y un valor vacío usa la ruta predeterminada.
- **Extensiones sin daemon:** Chromium y Firefox muestran un mensaje localizado específico en lugar de un error desconocido.
- **Capturas Xvfb:** la captura espera el render node hasta seis segundos por imagen y conserva los errores reales; el test separa las capturas 200 ms.

## Deuda aceptada para Fase 1

El límite global no se extiende a torrent ni a medios. Las hipótesis sobre pausa/reanudación de esos flujos y el tratamiento de manifiestos HLS/DASH requieren investigación y pruebas; no se modificaron en esta release. Las ramas de reintento y timeout de capturas tampoco se forzaron de forma determinista: un gate verde no demuestra ausencia total de intermitencias.

## Instalación

```bash
sudo apt install ./segmenta_0.3.1-1_amd64.deb
systemctl --user daemon-reload
systemctl --user enable --now segmenta.socket
segmenta hello
```

El paquete incluye las extensiones para Chromium y Firefox; también están disponibles por separado como `segmenta-extension-0.3.1.zip` y `segmenta-firefox-0.3.1.zip`.
