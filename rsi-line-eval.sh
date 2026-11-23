#!/usr/bin/env bash
# Evaluador del loop dream-rsi — stdout-number (solo el número, menor = mejor).
# Score: ms de la fila | search `line:(paragraph 3 alias)`, 50000 notes | de perf_budget.
# PROTEGIDO (protected_assets): el loop jamás lo edita; es propiedad del gate.
# Baseline medido 2026-11-21: 22.50 ms (FEATURES.md desactualizado al respecto).
# Fail-closed: sin fila o sin número → exit 1 sin stdout.
set -u

cd /home/diez/proyectos/RustSidian || exit 1

row=$(cargo run --release -p rustsidian --example perf_budget -- \
  --dir /tmp/rsi-perf-vaults --sizes 50000 2>/dev/null | \
  grep -F 'line:(paragraph 3 alias)')

if [ -z "$row" ]; then
  echo "EVALUATOR_FAIL: row 'line:(paragraph 3 alias)' not found" >&2
  exit 1
fi

ms=$(printf '%s' "$row" | awk -F'|' '{print $3}' | grep -oE '[0-9]+([.][0-9]+)?' | head -1)

if [ -z "$ms" ]; then
  echo "EVALUATOR_FAIL: no numeric ms in row" >&2
  exit 1
fi

echo "$ms"
