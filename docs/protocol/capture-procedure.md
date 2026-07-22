# Capturing Cameo 5 Alpha USB traffic

The vendor's cutting software runs on Windows/macOS only, so capture on one of those.

## Windows (primary)
1. Install Wireshark with the USBPcap component.
2. Plug in the Cameo 5, open the vendor's cutting software.
3. In Wireshark pick the USBPcap interface. Start capture.
4. In the software: load a 20mm square, Send. Stop capture when the machine finishes.
5. Save as `tools/capture/samples/cameo5-square-<date>.pcapng`.

## macOS (alternative)
1. `sudo ifconfig XHC20 up` then select the USB interface in Wireshark.
2. Same steps 2–5.

## Export bulk OUT payloads to a decoder fixture
Host→device bulk transfers carry the commands. Export their payload bytes:

    tshark -r tools/capture/samples/cameo5-square-<date>.pcapng \
      -Y 'usb.endpoint_address.direction == 0 && usb.transfer_type == 0x03' \
      -T fields -e usb.capdata | tr -d ':\n' > /tmp/cameo5-square.hex

`usb.transfer_type == 0x03` is bulk; direction 0 is OUT (host→device). Keep the
`.hex` as the committed fixture for the decoder.

## After capture

Feed the exported hex to the decoder to read the command stream:

    cd tools/capture && python3 -c "from decode import decode_records; \
      print('\n'.join(decode_records(open('/tmp/cameo5-square.hex').read())))"

Then:
- Trim a short representative slice into `tools/capture/samples/cameo5-square.hex`
  (the real fixture; replaces the committed `cameo5-square.SYNTHETIC.hex` placeholder).
- **Validate** the decoded stream against `silhouette-cameo5.md` (ported from
  `inkscape-silhouette`): expect ESC EOT init, `FG`, then `M`/`D` geometry in
  `(y,x)` at 20 units/mm.
