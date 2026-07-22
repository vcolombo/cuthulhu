# SPDX-License-Identifier: GPL-3.0-or-later
"""Build a minimal square cut job in the Cameo 5 (Alpha) GPGL dialect and send it
over the raw USB bulk endpoint. Command syntax mirrors
docs/protocol/silhouette-cameo5.md (ported from inkscape-silhouette, GPL-2.0+) —
update both together.

Cameo 5 Alpha: USB 0x3844:0x0001, bulk OUT endpoint 0x01. Coordinates are (y,x)
in device units at 20 units/mm; commands are ETX(0x03)-terminated; device init is
ESC(0x1b) EOT(0x04).
"""

import argparse
import sys

VENDOR_ID = 0x3844      # Cameo 5 Alpha   [src: Graphtec.py L128 (GPL-2.0+)]
PRODUCT_ID = 0x0001     # Cameo 5 Alpha   [src: Graphtec.py L142 (GPL-2.0+)]
EP_OUT = 0x01           # bulk OUT        [src: Graphtec.py L577 (GPL-2.0+)]
ETX = b"\x03"
INIT = b"\x1b\x04"      # ESC EOT = initialize  [src: Graphtec.py L174, L784 (GPL-2.0+)]


def build_square_job(size_mm: float, steps_per_mm: float = 20) -> list[str]:
    """GPGL geometry records for a closed square from the origin. M = move
    (pen up), D = draw (pen down). Coordinates are (y,x) in device steps."""
    s = round(size_mm * steps_per_mm)
    # path in (x,y): (0,0)->(x,0)->(x,y)->(0,y)->(0,0); encoded as (y,x)
    return ["M0,0", f"D0,{s}", f"D{s},{s}", f"D{s},0", "D0,0"]


def build_job(size_mm: float = 20.0, speed: int = 10, pressure: int = 10,
              tool: int = 1, steps_per_mm: float = 20.0) -> list[str]:
    """Full cut job: select tool, set speed + pressure, cut the square, feed out.
    Command syntax per docs/protocol/silhouette-cameo5.md."""
    return (
        [f"J{tool}", f"!{speed},{tool}", f"FX{pressure},{tool}"]
        + build_square_job(size_mm, steps_per_mm)
        + ["SO0", "FN0"]
    )


def serialize(records: list[str], terminator: bytes = ETX) -> bytes:
    """Join records into the ETX-framed wire byte stream (trailing ETX each)."""
    return b"".join(r.encode("ascii") + terminator for r in records)


def build_wire(**job_kwargs) -> bytes:
    """Full on-wire payload: ESC EOT init followed by the framed job."""
    return INIT + serialize(build_job(**job_kwargs))


def send(payload: bytes, vendor_id: int = VENDOR_ID, product_id: int = PRODUCT_ID) -> int:
    """Bulk-write payload to the Cameo. Requires pyusb + the physical device.
    Returns bytes written."""
    import usb.core  # lazy: unit tests and --dry-run don't need pyusb
    import usb.util
    dev = usb.core.find(idVendor=vendor_id, idProduct=product_id)
    if dev is None:
        raise RuntimeError(
            f"Cameo not found ({vendor_id:#06x}:{product_id:#06x}). "
            "Plugged in and powered? On Linux you may need udev permissions.")
    try:
        if dev.is_kernel_driver_active(0):
            dev.detach_kernel_driver(0)
    except NotImplementedError:
        pass  # not all backends implement this; harmless
    dev.set_configuration()
    usb.util.claim_interface(dev, 0)
    try:
        return dev.write(EP_OUT, payload)
    finally:
        usb.util.release_interface(dev, 0)


def main() -> None:
    ap = argparse.ArgumentParser(description="Cut a square on a Silhouette Cameo 5 Alpha.")
    ap.add_argument("--size-mm", type=float, default=20.0)
    ap.add_argument("--speed", type=int, default=10)
    ap.add_argument("--pressure", type=int, default=10)
    ap.add_argument("--tool", type=int, default=1, help="tool holder (1 or 2)")
    ap.add_argument("--steps-per-mm", type=float, default=20.0)
    ap.add_argument("--dry-run", action="store_true",
                    help="print the wire bytes as hex; do not open USB")
    args = ap.parse_args()

    payload = build_wire(size_mm=args.size_mm, speed=args.speed,
                         pressure=args.pressure, tool=args.tool,
                         steps_per_mm=args.steps_per_mm)
    if args.dry_run:
        sys.stdout.write(payload.hex() + "\n")
        return
    n = send(payload)
    print(f"sent {n} bytes to {VENDOR_ID:#06x}:{PRODUCT_ID:#06x}", file=sys.stderr)


if __name__ == "__main__":
    main()
