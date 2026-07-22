# SPDX-License-Identifier: GPL-3.0-or-later
"""Generic USB-payload decoder: split a byte stream into records and render
them as printable ASCII. Command *meanings* live in docs/protocol/ (ported
from the GPL drivers), not inferred here — this stays dialect-agnostic."""


def decode_records(hex_str: str, terminator: int = 0x03) -> list[str]:
    data = bytes.fromhex(hex_str.strip())
    out: list[str] = []
    for chunk in data.split(bytes([terminator])):
        if not chunk:
            continue
        rendered = "".join(
            chr(b) if 0x20 <= b < 0x7F else f"\\x{b:02x}" for b in chunk
        )
        out.append(rendered)
    return out
