# Seguridad en MetaMind

Auditoría de `lane/mm-sec`. Cada arreglo tiene test; los comandos del final reproducen el veredicto.

## Modelo de amenazas

- El usuario es dueño de la máquina y de la bóveda: no se defiende contra un proceso local con sus mismos permisos.
- **El contenido de la bóveda es hostil.** Notas, `[[links]]`, `.canvas`, `.excalidraw`, front matter, LaTeX y consultas llegan de bóvedas clonadas o compartidas: no pueden escribir fuera de la bóveda, ni colgar la app con un archivo malformado, ni elegir qué binario se ejecuta.
- El resto del escritorio no colabora (portapapeles ajeno, `metamind://` de otra app, descargas corruptas) y la red se usa sólo para lo que el usuario pide.

## Superficie

- **Archivos** (`.md`, `.canvas`, `.excalidraw`, `.base`, adjuntos): parsear y renderizar, nunca salir de la bóveda.
- **`metamind://`** (`uri.rs`): `new` rechaza nombres con `:`, absolutos o con segmento `..`; `open?path=` acepta rutas absolutas a propósito (así otra app abre un archivo) y por eso las rutas de la bóveda se tratan como hostiles.
- **Plugins** (`.obsidian/plugins/<id>/`, wasm/rhai/js): permisos de `metamind.permissions` y todo camino al disco por `ext_api::guard_path`/`safe_rel` (sin `..`, `\`, `:`, absolutos, ocultos ni symlinks que salgan); no se cargan si no están habilitados en esa bóveda. **Wasm** (wasmi, intérprete puro): el único import es `env.host_call`, sin WASI, fs, red, reloj ni entorno, con techos de tamaño, memoria, *fuel*, llamadas y fecha límite; pasarse es un trap.
- **Red**: `ureq` (IA) con rustls + `webpki-roots` y validación de certificados por defecto; la clave va como `Bearer` sólo al `base_url` configurado y no se reenvía en redirecciones. `curl` con TLS por defecto, `-q` y `--proto-redir`.
- **Procesos y portapapeles** (`ffmpeg`, `ffprobe`, `yt-dlp`, `tar`, `curl`, openers; `CF_DIB`, `CF_HDROP`, `text/uri-list`): todo por `argv`, nunca por shell, y los datos son de otro programa.

## Qué se endureció

**Alto**

1. **Path traversal** (`vault-core/src/store.rs`): `abs()` hacía `root.join(rel)` sin mirar `rel` y `read()` no normalizaba, así que un `[[../../etc/passwd]]` crafted, una URI o un plugin leían y escribían fuera de la bóveda. Ahora `abs()` sólo usa componentes normales y todo lo que toca disco pasa por ahí. Tests: `../../etc/passwd`, `/etc/passwd`, `..\..\Windows\…`, `C:/x`, `//server/share` y la lectura real de un archivo del directorio padre.
2. **`metamind://new` escribía fuera en Windows** (`uri.rs`): `name=C:evil.md` sobrevivía al chequeo de `..` y `vault.join()` lo resolvía como ruta de unidad. Test con `../`, `%2e%2e%2f`, `..%5C`, `C:`, `/etc/` y `\\server\share`, verificando que no se creó nada junto a la bóveda.
3. **Lectura fuera de bloque en el portapapeles** (`clipboard.rs`): `get()` escaneaba el `CF_UNICODETEXT` hasta el cero sin límite, y un dueño hostil que no lo termina hacía leer más allá del bloque (UB). Se acota con `GlobalSize`.
4. **El portapapeles quedaba tomado:** `set_png()` hacía `put(...)?` dentro del bloque `unsafe` y se saltaba el `CloseClipboard()`, bloqueándolo para todo el sistema. El cierre vive en el `Drop` de un guard que `put()`/`bytes()` reciben: la precondición la impone el tipo.
5. **Pánico con LaTeX sin cerrar** (`math-core/src/parse.rs`): `raw_group()` avanzaba una posición de más con un grupo sin terminar y el token siguiente indexaba fuera del slice (`$\begin{matrix` cerraba la app). Lo encontró el fuzz.

**Medio**

6. **`CF_DIB` hostil** (`clipboard.rs`): ancho/alto de 2^31, paleta de 4 G colores o header absurdo desbordaban `stride * h`; ahora se calcula con `checked_*` y es error de lectura.
7. **`yt-dlp` aceptaba cualquier URL** (`youtube.rs`): `is_youtube_url()` exige `http`/`https`, sin `user@`, y host de YouTube; `resolve()` no lanza nada que no pase el filtro.
8. **Tar-slip** (`update.rs`): antes de `tar -xf` se listan los miembros y uno que salga de la carpeta se rechaza.
9. **`wasmi` 2.0.0 estaba yanked** → 2.0.2 (mismo API).

**Bajo**

10. **`cargo deny` fallaba** por la licencia `CDLA-Permissive-2.0` de `webpki-roots` y una allowance sin usar.
11. `CF_HDROP` descarta nombres con longitud reportada mayor de 32 768; `to_dib()` satura en vez de desbordar.

## Aceptado a propósito (documentado, no corregido)

- **La clave de IA está en texto plano en `.obsidian/app.json`**: es el estándar de la bóveda local (Obsidian hace lo mismo) y viaja con la bóveda si se sincroniza. Usá una clave con límite de gasto.
- **Un plugin sin `metamind.permissions` recibe todos los permisos** (compatibilidad con Obsidian); hay que habilitarlo a mano en esa bóveda, y un plugin wasm sin permisos arranca sólo con lectura + UI.
- **`ffmpeg` recibe la URL `http(s)` que pide una nota** (video remoto: ese host ve la IP del usuario; otros esquemas se rechazan). **Los symlinks de la bóveda se siguen** al indexar archivos; `canonicalize` sólo se exige donde la ruta viene de fuera. **`mmap.rs`** (el único `unsafe` de `vault-core`) puede dar SIGBUS si otro proceso trunca el caché.
- **La actualización confía en los releases de GitHub**: TLS y el SHA256 del mismo release protegen de una descarga corrupta o de un mirror, no del dueño del repositorio. El **IPC local** (TCP loopback, token de 128 bits en un archivo `0600`) es del mismo usuario.
- **Cinco crates sin mantenimiento** (`smartstring`, `ttf-parser`, `zvariant`, `zvariant_derive`, `zbus_names`) llegan por `rhai` y `accesskit`; están en `[advisories].ignore` con el motivo.

## `unsafe`

`unsafe_code = "deny"` en `[workspace.lints.rust]` y las 17 crates heredan `[lints] workspace = true`, así que la regla es
efectiva en todo el árbol. Las excepciones son módulos con `#[allow(unsafe_code)]` y cada bloque lleva su comentario
`SAFETY`: `clipboard.rs` (FFI Win32), `datetime.rs`, `ospref.rs`, `main.rs`, `video.rs` (`libc::kill`), `vault-core/mmap.rs`
y `search-core/benches`.

## Cómo verificar

```sh
cargo audit                      # 5 avisos, todos "unmaintained" y justificados en deny.toml
cargo deny check                 # advisories, bans, licenses y sources: ok
cargo test -p vault-core && cargo test -p metamind && cargo test -p plugin-host-wasm
cargo clippy --workspace --all-targets
cargo check --target x86_64-pc-windows-gnu -p metamind   # el unsafe de Win32 sólo compila ahí
```

`crates/app/tests/malformed_input.rs` es el fuzz ligero: cada prefijo (en frontera de carácter) de 26 documentos hostiles
pasa por todos los parsers que leen archivos (`.excalidraw`, `.canvas`, dataview, búsqueda, markdown, front matter, LaTeX,
mermaid, URIs) y exige error o valor por defecto, nunca pánico. Así se encontró el de `math-core`.
