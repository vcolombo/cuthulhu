# SPDX-License-Identifier: Apache-2.0
"""Emit an HPGL program for a square, per docs/protocol/gcc-hpgl.md."""


def hpgl_square(size_mm: float, units_per_inch: int = 1016) -> str:
    u = round(size_mm / 25.4 * units_per_inch)
    return (
        "IN;PU0,0;"
        f"PD0,{u};PD{u},{u};PD{u},0;PD0,0;"
        "PU;"
    )
