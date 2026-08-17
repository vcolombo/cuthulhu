# Cuthulhu

Cuthulhu turns a vector design into the byte stream a craft cutting machine understands, through
a desktop app and a CLI that share one planning path.

## Language

### The design

**Document**:
The design being cut — a tree of Nodes plus the artboard they sit on and the machine they are
intended for.
_Avoid_: file, project, canvas, scene

**Node**:
One entry in a Document's tree: either a shape with an outline, or a container holding other
Nodes. Carries its own transform, its paint, and its CutLineType.
_Avoid_: element, object, layer, item

**Delta**:
A change to a Document, expressed so that its inverse can be derived. Every edit is a Delta, and
undo is applying the inverse.
_Avoid_: patch, diff, change set, mutation

**Trace**:
Converting a bitmap image into outlines that can be cut. A traced result enters the Document as
ordinary Nodes.
_Avoid_: vectorize, autotrace, raster conversion

**TraceControls**:
What a caller asks a Trace for, in the units a person setting them thinks in — mode, speckle,
smoothing, detail, colors. Ranges and defaults are stated once, in `trace::CONTROLS`. A higher
`detail` value yields more detail, the opposite direction from vtracer's `length_threshold`, which
never leaves the `trace` crate.
_Avoid_: options, params, settings (Settings is the machine's, not the tracer's), vtracer parameter
names

### Planning a cut

**DocumentPass**:
Every shape in a Document cut in a single run of the blade, together with the PassKey that says
which pass it is. What its shapes share is whatever the Grouping asked for — a colour, a material
preset — or nothing but the operator's request, for the single pass. Which shapes are cut at all
is their CutLineType's business, not their paint's.
_Avoid_: ColorPass (retired with #148), layer, colour group, batch

**PassKey**:
What a DocumentPass is called: `all` for the single pass, a colour, or a MaterialPreset id, each
with one canonical spelling (`color:ff0000ff`, `preset:cameo5-htv`) that the CLI, the IPC payloads
and the cut dialog all use. Absence is its own token — `no-color` for a shape with no visible
paint, `no-preset` for one resolving to no material — never a reserved value inside a mode,
because a preset id is an unrestricted operator string and one called `none` would collide.
_Avoid_: pass id, pass name, colour

**Grouping**:
How a Document's cut shapes are split into DocumentPasses — by stroke colour, fill colour,
stroke-else-fill, material preset, or not at all. A request, not a property of the Document: the
same Document plans differently under two Groupings, so the mode travels with every cut request,
and the cut dialog holds it with the rows it produced.
_Avoid_: mode, split, pass strategy

**PresetAssignment**:
What a Node says about its material: inherit from the nearest assigned ancestor, no material at
all, or a named MaterialPreset. Resolved while planning rather than stored, so moving a Node
between Layers changes what it is cut with and nothing has to be rewritten.
_Avoid_: material, preset id, optional preset

**CutLineType**:
Whether a Node is cut, and how — `Cut` or `NoCut` today. An explicit attribute of the Node
rather than something read off its stroke, defaulted to `Cut` at import.
_Avoid_: cut style, cuttable flag, cut attribute

**DocumentPasses**:
Every DocumentPass a Document contains, in the order they were first encountered. An inventory of
what *could* be cut — nothing has been chosen, checked, or configured yet.
_Avoid_: planned cut, plan

**PassSelection**:
A request to cut one DocumentPass, naming it by its PassKey and saying which Settings to use.
Their order is the order the passes are cut, and a pass nobody selects is not cut.
_Avoid_: configured pass, enabled pass

**CutPlan**:
The finished, validated result: the selected passes, in cut order, each already flattened into a
Job. Producing one is the only way to reach a machine.
_Avoid_: plan, document passes

**Preflight**:
The refusal rules a cut must satisfy before any bytes are produced — geometry that is finite, in
bounds, and non-degenerate; Settings inside the machine's range; a Document meant for the
machine that is plugged in; an output that fits in memory.
_Avoid_: validation, checks, linting

**Stale plan**:
A cut requested against a Document that has since changed. Refused rather than cut, because the
operator approved geometry that no longer exists.
_Avoid_: revision mismatch, dirty document

### Settings and materials

**Settings**:
How hard and how fast to cut, and how many times to repeat a pass. What a machine needs beyond
the geometry itself.
_Avoid_: parameters, options, config

**MaterialPreset**:
Named Settings for a particular material on a particular machine — vinyl on a Cameo, card on a
Puma. Some ship with the app; the rest are the operator's own.
_Avoid_: profile, material profile, recipe

**SettingsOverride**:
Settings an operator typed in for one pass, taking precedence over whatever MaterialPreset that
pass selected. Any field left unset defers to the preset.
_Avoid_: custom settings, manual settings

### Machines

**MachineProfile**:
The identity and physical extent of a machine model — what it is called and how large an area it
can cut.
_Avoid_: device profile, machine config, machine info

**MachineCaps**:
What a machine model can be told and what it needs from the operator: whether it honours speed
and force, and whether it requires a human to confirm each pass has finished.
_Avoid_: features, capabilities flags, support matrix

**Driver**:
The translator for one machine dialect — it turns a Job into that machine's bytes and knows how
to open, park and close a cutting session.
_Avoid_: backend, plugin, encoder, adapter

**Transport**:
The wire to a physical machine, over USB or a serial port. Carries bytes and knows nothing about
what they mean.
_Avoid_: connection, channel, port, link

**Cut Host**:
A machine that owns Transports to one or more cutters and runs Jobs on them on behalf of remote
clients. A Cut Host owns the cut: a client may detach mid-Job and the Job continues.
_Avoid_: proxy, server, relay, bridge — all four name something that forwards, which this does not.

**Job**:
The geometry for one pass, as flat polylines in millimetres, together with the Settings to cut it
with. What a Driver encodes.
_Avoid_: task, work item, payload

**Pass**:
One run of the blade over the material. More than one exists because a design in several colours
needs the operator to change tool or material between them — though a caller may ask for a single
pass outright and cut everything in one go.
_Avoid_: run, cycle, layer

**CutStatus**:
Where a cut has got to and what the operator may do next — the phase it is in, how the last one
ended, which of cut/cancel/resume/confirm are legal, which Pass of how many, how many bytes of it
have been sent, and the reason if it failed. The only thing a Driver's caller is told about a cut;
the states behind it are not anybody else's business, and a caller that keeps its own memory of
them has gone wrong.
_Avoid_: device state, state machine, progress

**Phase**:
What a machine is doing right now — idle, sending, cancelling, awaiting an operator, or failed.
Says nothing about how a previous cut turned out; that is what an Ended is for.
_Avoid_: status, state

**Ended**:
How the last cut finished — completed or cancelled — or nothing at all if no cut has been
attempted since the machine was connected. Separate from Phase because a finished machine and an
untouched one are both idle, and every caller that had to tell them apart invented its own memory
to do it.
_Avoid_: result, outcome, done
