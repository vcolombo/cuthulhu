# SPDX-License-Identifier: GPL-3.0-or-later
from send_raw import build_square_job, build_job, serialize, build_wire


def test_build_square_job_is_closed_path():
    job = build_square_job(size_mm=20, steps_per_mm=20)
    assert job[0] == "M0,0"                       # pen-up move to origin
    assert any(r.startswith("D") for r in job)    # has draw (pen-down) moves
    assert job[-1] == "D0,0"                       # closes back at origin


def test_square_scales_mm_to_device_steps():
    # 20mm * 20 steps/mm = 400 steps
    assert "D400,400" in build_square_job(size_mm=20, steps_per_mm=20)


def test_serialize_etx_frames_each_record():
    assert serialize(["M0,0", "D0,400"]) == b"M0,0\x03D0,400\x03"


def test_build_wire_starts_with_esc_eot_init():
    wire = build_wire(size_mm=20)
    assert wire.startswith(b"\x1b\x04")            # ESC EOT initialize
    assert wire.count(b"\x03") == len(build_job(size_mm=20))  # one ETX per record


def test_full_job_sets_tool_then_geometry():
    job = build_job(size_mm=20, tool=1)
    assert job[0] == "J1"                          # tool select first
    assert job[-1] == "FN0"                        # feed-out last
