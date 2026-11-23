# Changelog

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
