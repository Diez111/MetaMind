#!/usr/bin/env bash
# What the tablet reports, without opening the app.
#
# Lists the tablets the kernel has and then prints the pen's pressure as it moves. Use it to tell
# "the app does not read the pen" from "the pen does not report": if this says 0 % while you press,
# the problem is the device or its permissions, not MetaMind.
#
# Permission: /dev/input/event* is root:input 0660. If this says EACCES, add your user to the
# `input` group and log in again, or write a udev rule for the tablet:
#   SUBSYSTEM=="input", ATTRS{idVendor}=="28bd", MODE="0660", GROUP="input"
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

if [ ! -r /proc/bus/input/devices ]; then
    echo "Este sistema no tiene /proc/bus/input/devices: el lápiz por evdev es solo Linux." >&2
    exit 1
fi

echo "Tabletas detectadas:"
awk 'BEGIN { RS=""; FS="\n" } /BTN_TOOL_PEN/ { name=""; handlers=""; for (i=1;i<=NF;i++) { if ($i ~ /^N: /) name=$i; if ($i ~ /^H: /) handlers=$i } print "  " handlers "  ·  " name }' /proc/bus/input/devices | sed 's/^H: Handlers=/  /; s/^N: Name=//'

# A tiny Rust program would need a build; awk on `evtest` output would need evtest. Both are
# optional, so the probe only reports what can be read without them.
if command -v evtest >/dev/null 2>&1; then
    echo
    echo "Mové el lápiz sobre la tableta (Ctrl+C para salir). La presión es ABS_PRESSURE:"
    exec evtest
fi
echo
echo "Instalá evtest para ver la presión en vivo: sudo apt install evtest"
echo "O abrí MetaMind → Ajustes → Lápiz, que muestra la misma barra."
