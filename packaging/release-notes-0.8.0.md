# Segmenta 0.8.0

Captura, hosters y control remoto. Los enlaces que antes había que bajar a mano ahora se
resuelven solos, el control remoto viaja cifrado y cada teléfono emparejado se puede revocar
por separado. La versión instalada **sigue siendo 0.7.0** hasta que este documento se publique:
no hay tag ni release todavía.

## Extractores nativos

Un enlace deja de ser una página web que se guarda con nombre de archivo:

| Servicio | Qué resuelve |
| --- | --- |
| **Google Drive** | archivos (incluido el intersticial de archivos grandes) y exportación de Docs/Sheets/Slides a Office |
| **MediaFire** | el enlace real del botón de descarga |
| **Rapidgator** | el enlace anunciado en la página, con aviso cuando caducó |
| **Internet Archive** | el archivo del ítem, elegido por su metadata |
| **Gofile** | el archivo más grande del share |
| **GitHub releases** | el asset publicado, con su **SHA-256** cuando GitHub lo publica |
| **Hugging Face** | modelos y datasets, con el **SHA-256** del objeto LFS |

Cuando el servicio cambia y el extractor ya no reconoce la página, la descarga **falla con un
motivo** en vez de guardar el HTML. Eso es deliberado: un `.html` con nombre de archivo se
descubre recién al abrirlo.

## FTP y FTPS segmentado

`ftp://` y `ftps://` se descargan con el mismo motor de segmentos que HTTP: sondeo `SIZE` y
`MDTM`, `REST` + `RETR` pasivo, `DiskWriter` por offsets y el tope por host de siempre.

- **TLS explícito e implícito**, con `PBSZ`/`PROT P` para que el canal de datos también vaya
  cifrado.
- La IP que el servidor anuncia en `PASV` **se ignora**: se conecta al host del control, que es
  lo que evita el rebote de FTP y los problemas detrás de NAT.
- La reanudación se valida con `MDTM` **antes** de emitir `REST`: si el archivo remoto cambió,
  se niega en vez de empalmar dos versiones.

## Captura masiva

- **Patrones**: `foto_[01-25].jpg` se expande con su relleno, hasta 1000 URLs; un rango mayor se
  rechaza diciendo cuántos pedía.
- **Rastreo de carpetas**: listados `Index of`, sin salir del host ni del subárbol, con cotas de
  profundidad, carpetas y archivos. Cuando una cota corta la lista, **se dice cuál**.
- **ZIP remoto**: el árbol interno se lista por `Range` leyendo sólo el final del archivo
  (ZIP64 incluido), sin bajar el `.zip` completo, y una entrada suelta se descarga y se verifica
  contra su CRC-32.
- **Link Grabber**: se pega una lista, el daemon dice qué va a encolar y qué rechaza —con el
  motivo de cada línea— y la ventana lo muestra antes de encolar.

## Control remoto

- **HTTPS con certificado local** y huella SHA-256. La clave se guarda con permisos `0600` y se
  repara si estaba suelta; el certificado cubre loopback y las IPs de la LAN.
- **Página móvil embebida** con estado, alta de enlaces, filtros y acciones por fila, en tema
  claro/oscuro y en es/en/pt.
- **Un token por dispositivo** y **códigos de emparejamiento** de ocho dígitos, un solo uso, diez
  minutos y cinco intentos. Revocar un teléfono no desconecta a los demás.

## Notas

- La versión sigue en **0.7.0**: este documento se publica cuando se decida cortar la release.
- DoT sigue sin estar incluido en este build y devuelve un error que lo dice; PAC no aplica
  porque evaluarlo requiere un intérprete de JavaScript que el proyecto no incorpora.
- La suplantación TLS (perfil `chrome`) continúa como *seam* sin C++: se rechaza con un error
  que nombra la feature faltante.
