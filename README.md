# cuthulhu

Free, open-source (GPLv3), cross-platform desktop cutting software for vinyl
cutters and craft plotters — a modern, no-lock-in alternative to the
proprietary software that ships with these machines.

> **Status: early.** This is a *protocol spike* — there is **no application and
> no GUI yet**. The code here is throwaway protocol-research tooling. The real
> product (a Rust core behind a Tauri UI) is not built yet. See
> `docs/superpowers/specs/` for the design and roadmap.

## Target machines

- **Silhouette Cameo 5 Alpha** (USB, GPGL) — protocol ported from `inkscape-silhouette`.
- **GCC Puma IV** (HPGL over serial/USB) — from public GCC and HP-GL documentation.
- Other non-Cricut HPGL/DMPL cutters to follow. Cricut is out of scope (closed platform).

## Layout

- `docs/protocol/` — machine protocol notes, with cited sources (ported GPL driver code + public docs).
- `tools/` — Python spike scripts: generic USB-payload decoder, HPGL/GPGL square generators, and USB/serial senders.
- `docs/superpowers/` — design spec and implementation plans.

## Try the spike tools

```sh
cd tools && python3 -m pytest            # unit tests

# cut a 20mm square on a GCC Puma IV (needs pyserial)
python3 tools/replay/send_serial.py --port /dev/ttyUSB0 --size-mm 20

# cut a 20mm square on a Silhouette Cameo 5 Alpha (needs pyusb)
python3 tools/replay/send_raw.py --size-mm 20
```

Both senders support `--dry-run` to print the wire bytes without touching hardware.

## License

GPL-3.0-or-later — see `LICENSE`. Reuses the GPL drivers `inkscape-silhouette`
and `robocut`; attribution and source citations live in `docs/protocol/`.
