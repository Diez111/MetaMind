# MetaMind (repo `RustSidian`)

- **Un solo lane de build: `scripts/dev.sh`.** Todo `cargo` (test/check/clippy/build/run/bench)
  se corre como `scripts/dev.sh test --workspace`, etc. El script fija
  `CARGO_TARGET_DIR=<repo>/target` (disco, 372 G libres) y ejecuta dentro del slice
  `pi-build.slice` (CPU tope, sin martilleo de disco) vía `~/.local/bin/build-capped`.
- **Por qué el target no va a `/dev/shm`**: este workspace no entra en los 14 G del tmpfs
  (ya había 11 G usados). El linker muere con
  `collect2: fatal error: ld terminated with signal 7 [Bus error]` al escribir los binarios
  de test, y el comando puede devolver **exit 0** si termina en un pipe. Verificado 2026-11-22:
  6 binarios sin linkear (`test video_card`, `test excalidraw`, `test excalidraw_view`,
  `test tree`, `example frame_bench`, `example expand_bench`) sin un solo `test result`.
- `scripts/dev.sh` corre `set -euo pipefail`: el código de salida del build es el que ves.
- Un crate suelto sí puede ir a tmpfs si se quiere (`build-capped cargo test -p calendar-core`),
  pero nunca el workspace entero.
- Baseline sano: `scripts/dev.sh test --workspace` → 738 verdes, 0 rojos, 1 ignorado (170 s).
  Si aparece un rojo nuevo, es del cambio que acabás de hacer.
- Reboot ⇒ `/dev/shm` se vacía ⇒ el primer build de la sesión es frío (minutos de CPU,
  cero martilleo de disco).
