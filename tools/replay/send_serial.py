# SPDX-License-Identifier: GPL-3.0-or-later
"""Send an HPGL program to the GCC Puma IV over serial. Serial params per
docs/protocol/gcc-hpgl.md (9600, 8 data bits, no parity, 1 stop recommended).
"""

import argparse

from hpgl import hpgl_square


def main() -> None:
    ap = argparse.ArgumentParser(description="Cut a square on a GCC Puma IV over HPGL serial.")
    ap.add_argument("--port", required=True, help="serial device, e.g. /dev/ttyUSB0 or COM3")
    ap.add_argument("--baud", type=int, default=9600)
    ap.add_argument("--size-mm", type=float, default=20.0)
    ap.add_argument("--dry-run", action="store_true",
                    help="print the HPGL program; do not open the port")
    args = ap.parse_args()

    program = hpgl_square(args.size_mm)
    if args.dry_run:
        print(program)
        return

    import serial  # lazy: --dry-run works without pyserial installed
    with serial.Serial(args.port, args.baud, bytesize=serial.EIGHTBITS,
                       parity=serial.PARITY_NONE, stopbits=serial.STOPBITS_ONE,
                       timeout=5) as ser:
        n = ser.write(program.encode("ascii"))
        print(f"sent {n} bytes to {args.port} @ {args.baud} 8N1")


if __name__ == "__main__":
    main()
