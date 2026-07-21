# SPDX-License-Identifier: Apache-2.0
"""Generic USB-payload decoder: split a byte stream into records and render
them as printable ASCII. Command *meanings* are documented by hand in
docs/protocol/, not inferred here (clean-room)."""


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
