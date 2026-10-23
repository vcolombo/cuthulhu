// SPDX-License-Identifier: GPL-3.0-or-later
// Writes an SVG of N small closed curves (default 5000) to stdout, for the viewport's manual
// frame-rate check. Generated rather than committed: the number has to be re-measurable, and a
// megabyte of SVG in history buys nothing a 20-line script does not.
//
//   node apps/desktop/ui/scripts/perf-svg.mjs > /tmp/perf.svg      # then Import it
const n = Number(process.argv[2] ?? 5000);
const cols = Math.ceil(Math.sqrt(n));
const pitch = 4;
const paths = [];
for (let i = 0; i < n; i++) {
  const x = (i % cols) * pitch;
  const y = Math.floor(i / cols) * pitch;
  // Curves rather than squares: parsing and stroking cost is in the curves.
  paths.push(`<path d="M${x} ${y + 1.5} C${x} ${y} ${x + 3} ${y} ${x + 3} ${y + 1.5} S${x} ${y + 3} ${x} ${y + 1.5}Z" stroke="#000" fill="none"/>`);
}
const size = cols * pitch;
process.stdout.write(
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}mm" height="${size}mm" viewBox="0 0 ${size} ${size}">\n${paths.join("\n")}\n</svg>\n`,
);
