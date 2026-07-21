# GCC Puma IV — HPGL command set

Source basis: public GCC user manuals and the public HP-GL standard. No GPL
driver code consulted. See `README.md` for provenance rules and citation format.

Primary sources:
- `[doc: GCC Puma III Series User Manual, https://www.synergy17.com/downloads/GCC/manuals/PumaIIIusermanual.pdf]` — English; Puma III and Puma IV share interface conventions. Cited below as **PumaIII-UM**.
- `[doc: GCC Puma IV Series User Manual, https://www.gccworld.com/ (Cutter → Puma IV)]` — official; cited as **PumaIV-UM**.
- `[doc: HP-GL / HP-GL/2 plotter language standard]` — public HP standard for the geometric commands (IN/PU/PD/PA/SP). Cited as **HP-GL**.

## Command language

The Puma accepts **HP-GL and HP-GL/2** format data directly; most cutting
software emulates HP-GL and the plotter cuts what it receives.
`[doc: PumaIII-UM, "Most cutting software packages are able to emulate HP-GL or HP-GL/2 commands… the cutting plotter"]`

The Puma series can also operate in **GP-GL**; language is selected on the
machine. For this driver we target **HP-GL**. `[doc: PumaIV-UM, MISC / command-language menu]`
UNCONFIRMED: exact MISC-menu path and label for HP-GL vs GP-GL selection on the Puma IV Alpha panel — confirm on the physical machine.

## Coordinate system / units

- One HP-GL **plotter unit = 0.025 mm = 0.00098 in**. `[doc: HP-GL]`
- This matches the Puma's stated **Software Resolution 0.025 mm (0.00098")**. `[doc: PumaIII-UM, Specifications]`
- Therefore **1016 plotter units per inch** (= 40 units/mm). This is the `units_per_inch=1016` default in `tools/replay/hpgl.py`.
- Mechanical Resolution is finer at **0.009 mm (0.00035")** `[doc: PumaIII-UM, Specifications]`, but the command grid is the 0.025 mm software unit.

## Geometric commands (HP-GL standard)

All `[doc: HP-GL]`. Commands are ASCII, parameters comma-separated, each command
terminated by `;`.

| Command | Meaning |
|---------|---------|
| `IN;` | Initialize / reset to default state. Send first. |
| `PU x,y;` | Pen Up move to absolute (x,y) in plotter units — travel without cutting. |
| `PD x,y[,x,y…];` | Pen Down move(s) to absolute point(s) — cut along the path. |
| `PA x,y;` | Plot Absolute: set absolute coordinate mode (PU/PD then take absolute coords). |
| `PR x,y;` | Plot Relative: subsequent coords are relative. (Not used by our generator.) |
| `SP n;` | Select Pen n. On a cutter, selects the tool/holder where multi-tool applies. |

Minimal square job our generator emits (`hpgl_square`):

```
IN;PU0,0;PD0,U;PDU,U;PDU,0;PD0,0;PU;
```

where `U = round(size_mm / 25.4 * 1016)`.

## Cut parameters (force / speed / quality / offset)

The Puma can accept **Force, Speed, Cutting Quality, and Offset** either from the
control panel only, or "via software" when set to **Accept setup command** mode.
`[doc: PumaIII-UM, "Accept setup command: To accept commands of the Force, Speed, Cutting Quality, and Offset only via software"]`

- Reliable path for v1: set force/speed/offset on the machine's **control
  panel**, send only geometry over HP-GL.
- UNCONFIRMED: the exact software command syntax GCC uses to set force/speed
  in "Accept setup command" mode (the manual references vendor `Esc.`-prefixed
  setup commands but does not give a full force/speed table). Capture or
  manual-appendix confirmation needed before driving cut parameters from
  software. Carry into the driver spec (sub-project 2).

## Serial interface (RS-232)

`[doc: PumaIII-UM, §2.4.3 RS-232 Interface]`

- Connector: RS-232C serial (also parallel/Centronics on some units).
- **Baud 9600.** Data/parity combinations the panel offers (format `baud, parity, data-bits, stop-bits`):
  - `9600, n, 8, 1` — 8 data bits, no parity, 1 stop (recommended default).
  - Also selectable: 7 data bits; parity n / o / e.
- Communication parameters are set on the machine via the **MISC** menu and must
  match the host serial settings.
- UNCONFIRMED: factory-default parity/data-bits out of the box, and flow
  control (hardware RTS/CTS vs none) — set explicitly on both ends and confirm
  against the physical Puma IV. `tools/replay/send_serial.py` currently defaults
  to 9600 with pyserial defaults (8N1, no flow control); confirm on hardware.

## Machine capability (context)

- Cutting force up to **500 g**, speed up to **1020 mm/s** (600 mm/s diagonal).
  `[doc: PumaIV-UM / GCC product spec]`

## Open questions carried to driver spec (sub-project 2)

- HP-GL vs GP-GL selection menu path on Puma IV Alpha.
- Software force/speed/offset command syntax ("Accept setup command" mode).
- Default serial parity/data-bits and flow-control requirement.
