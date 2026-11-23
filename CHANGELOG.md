# Changelog

## 0.7.7

- **Un enlace de YouTube se ve acá, no en el navegador.** La app tenía el reproductor nativo y el
  resolvedor (`yt-dlp`), pero un enlace `[texto](https://youtube.com/watch?v=…)` mandaba al
  navegador: la ruta del player existía solo para las tarjetas de video. Ahora el enlace resuelve
  con `yt-dlp` y se reproduce en el mismo player que un archivo local; si `yt-dlp` no está, avisa y
  abre el navegador. En esta máquina están `yt-dlp` y `ffmpeg`, así que es el camino que se toma.


## 0.7.6

- **Abrir un video lo reproduce.** Un clic en un `.mp4` (desde el árbol, un enlace o la paleta)
  contestaba «solo se pueden abrir notas Markdown en esta compilación», con un reproductor nativo
  al lado. Ahora el archivo va al reproductor; si falta el decodificador, el aviso dice eso.
- **Las tablas se leen.** El encabezado salía a 0,75 em del cuerpo (unos 11 px contra 15) y las
  celdas iban apretadas: ahora la tabla es 0,95 em con 8×12 de relleno, como el texto alrededor.
- **Calendario de la barra.** Se fue el botón «Crear evento» del estado vacío (el «+» del día ya
  hace eso), los dos iconos del día no tienen fondo hasta el hover y los días de otro mes quedan
  más apagados, con sus puntos atenuados.


## 0.7.5

- **Servidores MCP como herramientas del copiloto.** `crates/mcp-client` habla el protocolo
  (JSON-RPC por stdin/stdout, `initialize`, `tools/list`, `tools/call`) sin `unsafe` ni runtime
  asíncrono, y la app los declara en `.obsidian/app.json`:

  ```json
  {"mcp": {"servers": [{"name": "filesystem", "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/me"]}]}}
  ```

  El primer turno que necesita herramientas arranca cada servidor, lee su lista y ofrece cada una
  como `mcp__<servidor>__<herramienta>`; una llamada va al servidor y su texto vuelve al modelo.
  Un servidor que no arranca se informa y se saltea, y el resto del turno sigue. El cliente se
  prueba contra un servidor real (`mock-mcp`, incluido en el crate) que habla el protocolo.


## 0.7.4

Tres superficies nuevas y el pulido que salió de mirar capturas renderizadas a tamaño real.

- **Tablero de finanzas.** Una nota con `tipo: finanzas` se dibuja como tablero: ingresos, gastos y
  balance del mes, una barra por mes en los últimos seis (ingreso y gasto) y las categorías del mes
  con su peso. Los movimientos se escriben a mano, una línea por movimiento:
  `- 2026-11-03 · supermercado · 45.200`, bajo un título `## Ingresos` o `## Gastos`. Los montos se
  leen como los escribe una persona (`1.250.000`, `1250,50`, `$45.200`) y se suman en centavos.
- **Diagrama de Gantt.** Una nota con `tipo: gantt` es un plan: `- Diseño · 2026-11-02 · 2026-11-08`,
  una fila por tarea, barra de inicio a fin, rombo para un hito, escala de meses y marca de hoy. Las
  tareas terminadas quedan en gris.
- **IA local sin clave.** Un perfil contra `localhost`/`127.0.0.1` (Ollama y compañía) ya no exige
  una clave de API: el panel se negaba a enviar y hacía imposible usar un modelo local. El preset
  de Ollama ya estaba; ahora funciona.
- **Pulido de interfaz, visto en capturas.**
  - Calendario de la barra: el estado vacío tenía el botón impreso **encima** del texto; el título
    del día se cortaba y las líneas de la grilla no se veían.
  - Editor de evento: el separador «a» entre los relojes se estiraba como si fuera la fecha.
  - Tableros: las columnas llenan el alto del panel (antes tres tarjetas flotando en el vacío).
  - Excalidraw: se fue el «Ajustar/100 %» duplicado del encabezado, el icono de goma es una goma y
    un relleno pastel ya no se vuelve azul marino sobre el papel oscuro.
  - Semana en el teléfono: degrada a la lista de sus siete días en vez de siete columnas de 48 px.
- **`examples/shots.rs`**: renderiza 17 superficies a 390×844, 1024×768 y 1280×800 en PNG. Es como
  se revisa el diseño de ahora en adelante: se mira el render, no la hoja de estilo.


## 0.7.3

Lo que se vio usando la 0.7.2, arreglado en la interfaz.

- **Calendario de la barra.** El mes es compacto (32 px por fila), el número queda centrado en su
  celda y los puntos van abajo; antes el número se iba a la izquierda y la grilla se comía el alto
  que necesita la lista del día.
- **Editor de evento.** Flechas de verdad en vez de los glifos `‹ ›`, un interruptor con perilla
  para «Todo el día» (era una píldora vacía), los relojes con iconos de menos y más, y la tarjeta
  pasa de 440 a 560 px en escritorio (92 % en móvil).
- **Tableros.** La barra tiene **Goma** como herramienta, y al terminar un trazo ya no queda
  seleccionado: ese era el marco punteado que aparecía al escribir. La paleta entra en filas de
  ocho colores y se fue el texto de ayuda que sobraba.
- **Diálogo de tablero nuevo.** 540 px, centrado, la carpeta en palabras («En la carpeta X») y cada
  plantilla con una vista previa de sus columnas y los nombres debajo.
- **Tablero Kanban.** Las columnas se centran cuando no llenan el ancho: tres columnas pegadas al
  borde izquierdo de una pantalla grande era lo que se veía como «todo chico y re vacío».
- **Dashboard.** Botón en la ribbon (estaba solo en la paleta de comandos).


## 0.7.2

- **Nuevo tablero, con diálogo.** `kanban:new` (el botón de la ribbon) ya no crea `Tablero.md` a
  ciegas: pregunta nombre, carpeta (la de la nota o la del tablero abierto) y plantilla — Básico,
  Proyecto (Pendiente/Haciendo/Bloqueado/Hecho) o Estudio (Por leer/Leyendo/Resumido/Repasado). Un
  nombre tomado suma sufijo en vez de pisar el archivo, `Escape` cancela, y las letras van al
  nombre, nunca a la nota de atrás.
- **Relleno rayado en los tableros.** `fillStyle` se leía y se ignoraba: todo salía sólido. Ahora
  `hachure` y `cross-hatch` se dibujan como líneas a 45° recortadas a la figura (una figura grande
  engrosa el paso en vez de llenar la lista de comandos), y el panel de estilo tiene su fila
  Sólido/Rayado/Cruzado. La elección queda en el archivo.
- **Vista grande del calendario.** `calview:open` abre una pestaña con Mes, Semana, Día y Agenda
  sobre los mismos eventos: el mes con chips, la semana con una columna por día y cada evento
  puesto por su hora, el día hora por hora y la agenda de los próximos 60 días. Las flechas mueven
  el mes, la semana o el día según la vista.
- **El gate del CI, limpio.** `clippy -D warnings` y `rustfmt --check` en todo el workspace; quedó
  también `scripts/dev.sh clean` para el target que llegó a 180 GB.


## 0.7.1

Tres cosas que la 0.7.0 dejó mal y se vieron al usarla.

- **El botón Estilo del tablero no hacía nada.** Abría el panel, pero lo agregaba antes del
  lienzo: el pintor llenaba su nodo encima y la tarjeta quedaba invisible. Ahora va después, con
  un test del orden de pintado y de que los tres grosores están ahí — de paso, el grosor de línea
  y los colores ya se pueden elegir.
- **Al dibujar aparecía un marco punteado.** Era la selección del trazo en curso. Se omite
  mientras se dibuja o se traza a mano; la banda, el movimiento y el pan conservan su marco.
- **Una nota del plugin de Excalidraw mostraba el JSON entero.** El dibujo vive en un bloque `%%`
  de la nota: ahora los comentarios se ocultan fuera del modo fuente, y si el bloque tiene un
  escenario de Excalidraw la nota se abre directamente como tablero. Al guardar se reemplaza solo
  el JSON dentro del bloque; el frente, la prosa y los marcadores quedan igual.
- **Target de build.** `scripts/dev.sh clean`: `cargo clean` a secas limpiaba el tmpfs del slice y
  dejaba 180 GB de artefactos viejos en el target de disco (los links tardaban minutos).


## 0.7.0

The version of the audit: the calendar can be written, the boards can be found, the pen has
pressure, the copilot has tools, and the layout stopped overflowing.

### Calendario

- **Eventos propios.** A `.metamind/calendar/events.json` in the vault, with all-day events, times,
  eight colours, a place and notes. Written atomically; a broken file is backed up next to itself
  and the calendar starts empty with a notice, and a key this build does not know survives a save.
- **Un editor de verdad.** Title, place, notes, day steppers, time steppers, all-day toggle and
  colours, with `Enter` to save, `Escape` to cancel and a two-step delete. A click on one of your
  events opens it; a click on a Google one shows its detail.
- **El día elegido se ve.** The hierarchy was inverted: today painted the strong violet and the
  selected day painted almost nothing. Today is now a coloured number; the selected day is a ring.
- **La grilla es una grilla.** One frame with a line between cells, the number at the top left, and
  chips with `09:30 Título` when the cell is wide enough. Clicking a day no longer moves the month
  under the pointer.

### Tableros

- **Excalidraw con una barra de verdad.** The tools and the style live in a bar across the top and
  a panel anchored to the corner, so nothing is pinned to a `top` that another element can outgrow.
  The reason the palette spilled out of the pane was that the layout engine did not implement
  `flex-wrap`: it does now.
- **El papel del tablero.** Dots, a grid, ruled lines or nothing, with three sizes, saved in the
  file's `appState` so Excalidraw opens it the same way.
- **El lápiz.** A new `crates/pen-input` reads the tablet from `evdev` (no `unsafe`, no
  dependency): a stroke keeps one pressure per point and the renderer draws it as a ribbon, and the
  other end of the pen erases. Ajustes → Lápiz shows the device, its state, the live pressure and
  the way out when the device cannot be read.
- **Kanban a la vista.** A button in the ribbon, and the board's format shown inside the board next
  to the template the parser reads.

### Copiloto

- **Herramientas.** The model can search, read, list, create and append notes, create a calendar
  event, add a Kanban card and open a note. The loop runs on the frame, where the vault lives,
  answers the model and asks again, and stops after four rounds. Paths are validated inside the
  vault, and the panel shows one row per function with what it did.

### UI, layout y render

- **`flex-wrap` de verdad** in `metamind-ui`: lines, per-line grow and shrink, `row-gap` between
  lines and `align-content`.
- **`Cmd::Stroke`**: a polyline whose width changes along it, drawn as one ribbon so a translucent
  stroke is composited once.
- **El repintado por daño dejó de mentir.** Masking an anti-aliased shape rounded its coverage by
  one level; a shape that fits inside the damaged rect is now drawn without a mask, and the test
  that compares a full repaint with a damaged one passes for the first time.
- **`clamp()` sobre CSS ya no puede tumbar la ventana.** A `max-height: calc(100% - 60px)` resolved
  to a negative number and the layout pass panicked.
- **El barrido de overflow cubre los tableros.** Excalidraw (with and without its bar, with the
  style panel), Kanban, Canvas, Bases and the calendar editor are checked at seven window sizes
  from 360×640 to 2560×1440. It found thirteen overflows the day it was turned on.

### Marca y proyecto

- **Un solo nombre: MetaMind.** The UI said MetMind, the README said MetaMind and the `.desktop`
  announced a `rustsidian` scheme. Twenty-four files fixed, the updater points at the fork, and the
  README opens with the fork's lineage.
- **`scripts/dev.sh`.** One lane for every build: the target on disk (this workspace does not fit
  in `/dev/shm`: the linker died with `SIGBUS` and the pipe still reported success) and
  `pipefail`, so a build cannot lie about failing.
- **Baseline medido.** 791 tests, 0 failures, 1 ignored, after the audit.

### Limitaciones que quedan

- Rotation, grouping, arrow bindings and embedded images are still out of the Excalidraw board.
- The calendar opens its own events; a Google event is read-only, as it should be.
- Pressure needs a Linux tablet the user can read (`input` group or a udev rule).
