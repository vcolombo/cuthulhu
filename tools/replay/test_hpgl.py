# SPDX-License-Identifier: GPL-3.0-or-later
from hpgl import hpgl_square

def test_hpgl_square_golden_25_4mm():
    # 25.4mm = 1 inch = 1016 plotter units. Square from origin.
    prog = hpgl_square(25.4)
    assert prog == "IN;PU0,0;PD0,1016;PD1016,1016;PD1016,0;PD0,0;PU;"

def test_hpgl_square_scales_mm_to_units():
    prog = hpgl_square(50.8)  # 2 inches -> 2032 units
    assert "PD2032,2032;" in prog
