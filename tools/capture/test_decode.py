# SPDX-License-Identifier: GPL-3.0-or-later
from decode import decode_records

def test_splits_on_terminator_and_renders_ascii():
    # "PU100,200" ETX "PD0,0" ETX  as hex
    hex_str = "5055313030" + "2c323030" + "03" + "5044302c30" + "03"
    assert decode_records(hex_str) == ["PU100,200", "PD0,0"]

def test_shows_nonprintable_as_hex_escape():
    # 0x1b (ESC) 'A' then terminator
    assert decode_records("1b4103") == [r"\x1bA"]

def test_ignores_trailing_empty_record():
    assert decode_records("5044302c3003") == ["PD0,0"]
