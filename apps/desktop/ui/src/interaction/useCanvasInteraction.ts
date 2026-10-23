// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type PointerEvent, type RefObject } from "react";
import { hitTest, type Bounds, type Scene } from "../render/hittest";
import { compose, IDENTITY, isIdentity, type Pt } from "../render/affine";
import type { Canvas2DRenderer } from "../render/Canvas2DRenderer";
import { applyMoves, applyOptimistic, gestureScene, type Matrix, type PendingPreview } from "./transform";
import {
  CSS_PX_PER_MM, CONTENT_FIT_MARGIN_PX, FIT_MARGIN_PX, IDENTITY_VIEW, ZOOM_STEP, fitView, minScaleFor, panBy,
  screenToWorld, wheelFactor, zoomAt, type Inset, type Size, type View,
} from "./viewport";
import { handleAt, selectionBox, type Box, type HandleKind } from "./selectionBox";
import { gestureMatrix, type Modifiers } from "./gesture";
import { boxBounds, keepLanded, SNAP_PX, snapMove, snapScale, snapTargets, type Guide, type Targets } from "./snap";
import { marqueeHits, marqueeSelection, normalizeRect, toggleId } from "./marquee";
import type { Move } from "./align";

// CSS px, divided by the view scale at use so they feel the same at every zoom.
const HANDLE_HIT_PX = 6;
const ROTATE_ZONE_PX = 18;
const HIT_TOL_PX = 3;
/** A press that travels less than this is a click: it clears the selection instead of banding. */
const CLICK_SLOP_PX = 3;
/** Some wheels report lines rather than pixels; one line is about one line of text. */
const LINE_PX = 16;

const CURSORS: Record<HandleKind | "pan", string> = {
  nw: "nwse-resize", se: "nwse-resize", ne: "nesw-resize", sw: "nesw-resize",
  n: "ns-resize", s: "ns-resize", e: "ew-resize", w: "ew-resize",
  rotate: "crosshair", move: "move", pan: "grab",
};

type Gesture =
  | { t: "pan"; last: Pt } // screen px
  // `ids` is what commits (a container moves its whole subtree); `shapes` is what that moves on
  // screen, so the preview can match the commit.
  // `targets` are gathered once at the press, since only the selection moves during a gesture;
  // `guides` are the snap lines of the current frame.
  | { t: "transform"; kind: HandleKind; box: Box; ids: number[]; shapes: number[]; start: Pt; m: Matrix; targets: Targets; guides: Guide[] } // world mm
  | { t: "marquee"; start: Pt; cur: Pt; additive: boolean }; // world mm

export type CanvasInteractionArgs = {
  canvasRef: RefObject<HTMLCanvasElement>;
  rendererRef: RefObject<Canvas2DRenderer>;
  scene: Scene;
  selected: number[];
  setSelected: (ids: number[]) => void;
  /** Every shape at or beneath the given ids: what a selection that includes a Group or Layer
   *  actually moves. The box, highlight and preview use it; the commit sends the ids themselves. */
  expand: (ids: number[]) => number[];
  artboard: Bounds | null;
  /** The mat the view fits and the zoom floor is taken from. Null until a document is open.
   *  Not the artboard: that is the machine bed, and fitting it leaves a 12 in sheet as a sliver. */
  mat: Bounds | null;
  /** Artwork bounds for Fit content. Read from a ref inside the fit effect, so an edit does not refit. */
  contentBounds: Bounds | null;
  /** Rulers, when shown. A zero inset matches the older fit exactly. */
  inset: Inset;
  /** Revision of the snapshot `scene` was built from; it rises with every successful refresh. */
  sceneRev: number;
  /** A refusal puts the preview back. An applied commit keeps it until its own snapshot has
   *  rendered, even if that refresh failed, because the backend then holds the new geometry.
   *  `requestedAt` is when the operator asked for it (`performance.now()`), not when it went
   *  out: an edit queued before a refusal must leave that refusal on screen, and one made after it
   *  must clear it, though both go out from the queue (Copilot on #301). */
  commit: (moves: Move[], requestedAt: number) => Promise<CommitOutcome>;
};

/** What a deferred edit reads when it is sent rather than when it was made: the tree that turns a
 *  Group into its shapes, and the bed. The commit ahead of a queued click can change either
 *  (Copilot on #301), and both are this render's, so they come from the hook's own latest values. */
export type SendTime = { expand: (ids: number[]) => number[]; artboard: Bounds | null };

/** "applied" carries the revision of the snapshot its refresh rendered, or null if that refresh
 *  failed or rendered nothing: the commit's preview stands in until a snapshot at least that new has
 *  rendered. */
export type CommitOutcome = { kind: "refused" } | { kind: "applied"; snapshotRev: number | null };

/** Which property field a queued edit came from: edits to one field supersede each other. */
export type PropertyField = "x" | "y" | "w" | "h";

/** Why document edits are refused for now, or null; see `CanvasInteraction.editsLock`. */
export type EditsLock = "loading" | "unread" | null;

type Handlers = {
  onPointerEnter: () => void;
  onPointerDown: (e: PointerEvent<HTMLCanvasElement>) => void;
  onPointerMove: (e: PointerEvent<HTMLCanvasElement>) => void;
  onPointerUp: () => void;
  onPointerCancel: () => void;
  onPointerLeave: () => void;
};

export type CanvasInteraction = {
  view: View;
  size: Size;
  cursor: Pt | null;
  requestFit: () => void;
  /** Fit the artwork with an 80 px margin. Import, add and trace call this; a plain edit does not. */
  requestFitContent: () => void;
  zoomIn: () => void;
  zoomOut: () => void;
  /** Draws the scene, selection and any live gesture. App calls it whenever the view, size, scene
   *  or selection changes, so those redraws cannot paint the committed scene over a gesture. */
  repaint: () => void;
  /** Set while Open or Reload loads ("loading"), and after a successful one until the loaded
   *  document's snapshot renders ("unread"): an edit made meanwhile would be dropped, and after the
   *  load the canvas still shows the old document. The panel disables its controls on it, so a click
   *  there does not vanish without a word (silent-failure-hunter and Copilot on #301). The two are
   *  told apart because "unread" outlasts a read that failed, and only a new read lifts it. */
  editsLock: EditsLock;
  /** The same, read when called: a keypress or click handler runs between renders, and a load
   *  started since the last one has already locked edits. */
  editsLockNow: () => EditsLock;
  /** The geometry the canvas shows: an unread or in-flight commit's preview, else the committed
   *  scene. Anything that computes a transform from current positions must read this one. */
  effectiveScene: Scene;
  /** The one way to commit a transform from outside a canvas gesture (the X/Y/W/H fields). The
   *  matrix is built from the effective scene when it is sent; while a commit is on the wire the
   *  edit waits behind it (the latest per field), so it never starts from a position the shape has
   *  left. */
  transformWith: (field: PropertyField, ids: number[], make: (scene: Scene) => Matrix | null) => void;
  /** The same, for a click that moves each unit differently (align, distribute) as one commit. A
   *  newer click under the same key replaces a queued one. */
  transformEach: (key: string, make: (scene: Scene, now: SendTime) => Move[]) => void;
  /** Runs `load`, which replaces the document (Open, Reload), with every edit held until it
   *  settles. Ids are reused, so an edit made against the old document would move a shape in the
   *  new one (Copilot on #301): one queued before the load, or one made while it runs, which used
   *  to go out at once. If the load succeeds those are dropped; if it is refused the document is
   *  still the one they were made on, so they go out as usual. */
  replaceDocument: (load: () => Promise<unknown>) => Promise<void>;
  handlers: Handlers;
};

function toCanvas(canvas: HTMLCanvasElement, clientX: number, clientY: number): Pt {
  const r = canvas.getBoundingClientRect();
  return { x: clientX - r.left, y: clientY - r.top };
}

/** The part of `box` under `p` at view scale `scale`, with the CSS-px sizes converted to mm in
 *  one place so the hover cursor and the press cannot disagree about what is under the pointer. */
function handleUnder(box: Box, p: Pt, scale: number): HandleKind | null {
  return handleAt(box, p, HANDLE_HIT_PX / scale, ROTATE_ZONE_PX / scale);
}

function isTyping(t: EventTarget | null): boolean {
  return t instanceof HTMLElement && (["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName) || t.isContentEditable);
}

export function useCanvasInteraction(args: CanvasInteractionArgs): CanvasInteraction {
  const { canvasRef, rendererRef, scene, selected, setSelected, expand, artboard, mat, contentBounds, inset, commit, sceneRev } = args;
  const [view, setViewState] = useState<View>(IDENTITY_VIEW);
  const [size, setSize] = useState<Size>({ w: 0, h: 0 });
  // ponytail: the readout re-renders App on every pointer move. Fine at today's App size; if a
  // profile ever shows it, feed a small readout component from a ref instead of state.
  // Held in screen px and converted on render: a pan or zoom moves the world under a still
  // pointer, and a world position stored at the last move would go stale (Copilot on #298).
  const [cursorScreen, setCursorScreen] = useState<Pt | null>(null);
  const [fitReq, setFitReq] = useState<{ kind: "mat" | "content"; n: number }>({ kind: "mat", n: 0 });
  // The fit effect must not depend on these objects: a new scene would refit content on every edit,
  // and a new inset object would do the same for a render that did not toggle the rulers.
  const contentRef = useRef(contentBounds);
  contentRef.current = contentBounds;
  const insetRef = useRef(inset);
  insetRef.current = inset;
  const matRef = useRef(mat);
  matRef.current = mat;
  // Native listeners are registered once and pointer handlers run between renders; both read
  // these rather than a render's closure, so they see the selection the user just made.
  const viewRef = useRef(view);
  const latest = useRef({ scene, selected, expand, commit, sceneRev, artboard });
  latest.current = { scene, selected, expand, commit, sceneRev, artboard };
  const pending = useRef<PendingPreview | null>(null);
  // `pending` is a ref so pointer handlers see it between renders, but `effectiveScene` is read by
  // React: the panel's buttons kept the bounds from before a preview until something else
  // re-rendered, and offered clicks that then did nothing (Copilot on #301). Bumped whenever
  // `pending` changes.
  const [, previewChanged] = useReducer((n: number) => n + 1, 0);
  // Set when a commit settles with edits queued behind it; the next render's effect drains them.
  const drainDue = useRef(false);
  useEffect(() => {
    if (loadedAtRev.current !== null && !stale()) loadedAtRev.current = null;
    if (!drainDue.current) return;
    drainDue.current = false;
    drain();
  });
  // One transform on the wire at a time. A gesture pressed while a commit is unanswered would build
  // its matrix on that commit's preview, and if the commit were refused it would land about the
  // wrong anchor. The window is one IPC round trip, so a press in it selects but does not drag.
  const inFlight = useRef(false);
  // While Open or Reload replaces the document: edits queue and nothing drains (`replaceDocument`).
  const replacing = useRef(false);
  // The commit on the wire, settled or not, so a load can wait for it (`replaceDocument`).
  const settling = useRef<Promise<void>>(Promise.resolve());
  // The scene revision when a load replaced the document. Until a newer snapshot renders, the
  // canvas still shows the old document, so an edit made there would send old bounds and an id the
  // new one may reuse (Copilot on #301); `stale()` holds until then, even if that refresh failed.
  const loadedAtRev = useRef<number | null>(null);
  const stale = () => loadedAtRev.current !== null && latest.current.sceneRev <= loadedAtRev.current;
  // Stable, since it reads only refs: App's key handler depends on it and would otherwise re-bind on
  // every render, which a pointer move causes.
  const editsLockNow = useCallback((): EditsLock => (replacing.current ? "loading" : stale() ? "unread" : null), []);
  // Property edits made while a commit is on the wire, in the order they were made.
  const queued = useRef(new Map<string, { make: (s: Scene, now: SendTime) => Move[]; at: number }>());
  const sendTime = (): SendTime => ({ expand: latest.current.expand, artboard: latest.current.artboard });
  // What gestures start from and what the renderer is put back to: the in-flight preview until a
  // snapshot replaces the scene it was built on, then the committed scene. CodeRabbit on #298.
  const current = () => ({
    scene: gestureScene(pending.current, latest.current.scene, latest.current.sceneRev),
    selected: latest.current.selected,
  });
  const gesture = useRef<Gesture | null>(null);
  const spaceHeld = useRef(false);
  const overCanvas = useRef(false);
  // Where the pointer last was on screen and which modifiers it held. A gesture follows the
  // document point under the pointer, so a pan or zoom under a still, held pointer must re-derive
  // it from these rather than keep the matrix of the last move (CodeRabbit on #298).
  const lastScreen = useRef<Pt | null>(null);
  const lastMods = useRef<Modifiers>({ shift: false, alt: false });
  // ⌘ (Ctrl off the Mac) held: drag without snapping. Read on every move, so it can be pressed mid-drag.
  const snapOff = useRef(false);

  const setView = useCallback((v: View) => {
    viewRef.current = v;
    setViewState(v);
  }, []);
  const requestFit = useCallback(() => setFitReq((r) => ({ kind: "mat", n: r.n + 1 })), []);
  const requestFitContent = useCallback(() => setFitReq((r) => ({ kind: "content", n: r.n + 1 })), []);
  // The floor follows the mat. Falling back to the bed only happens before the mat exists.
  const zoomTarget = mat ?? artboard;
  const minScale = useMemo(
    () => (zoomTarget && size.w > 0 && size.h > 0 ? minScaleFor(zoomTarget, size, FIT_MARGIN_PX, inset) : 0),
    [zoomTarget, size, inset],
  );
  const zoomBy = useCallback(
    (screen: Pt, factor: number) => setView(zoomAt(viewRef.current, screen, factor, minScale)),
    [minScale, setView],
  );
  const centre = useCallback((): Pt => ({ x: size.w / 2, y: size.h / 2 }), [size]);
  const zoomIn = useCallback(() => zoomBy(centre(), ZOOM_STEP), [centre, zoomBy]);
  const zoomOut = useCallback(() => zoomBy(centre(), 1 / ZOOM_STEP), [centre, zoomBy]);

  // The one place the renderer is told what to show. Pointer moves call it, and so does App's
  // effect on any view, size, scene or selection change: a separate redraw from the committed scene
  // painted over a live drag on a mid-gesture pinch, and over the in-flight preview on a selection
  // change (code-reviewer on #298). With no gesture it also puts back a stranded preview.
  const repaint = useCallback(() => {
    const r = rendererRef.current;
    if (!r) return;
    const { scene: s, selected: sel } = current();
    const g = gesture.current;
    if (g?.t === "transform") {
      r.setScene(applyOptimistic(s, g.shapes, g.m));
      r.setSelection(g.shapes);
      r.setOverlay({ box: { ...g.box, frame: compose(g.box.frame, g.m) }, marquee: null, guides: g.guides });
    } else {
      const shapes = latest.current.expand(sel);
      r.setScene(s);
      r.setSelection(shapes);
      r.setOverlay(
        g?.t === "marquee"
          ? { box: null, marquee: normalizeRect(g.start, g.cur), guides: [] }
          : { box: selectionBox(s, shapes), marquee: null, guides: [] },
      );
    }
    r.draw();
  }, [rendererRef]);

  // Re-derives a live transform or marquee from the pointer under the current view.
  const follow = useCallback(() => {
    const g = gesture.current;
    const screen = lastScreen.current;
    if (!g || g.t === "pan" || !screen) return;
    const p = screenToWorld(viewRef.current, screen);
    if (g.t === "marquee") {
      gesture.current = { ...g, cur: p };
      return;
    }
    // Snapping moves the pointer, not the matrix, so Shift, Alt and the commit see an ordinary
    // drag that happened to end on a line. Rotation keeps its own 15° steps.
    let point = p;
    let guides: Guide[] = [];
    if (g.kind !== "rotate" && !snapOff.current) {
      const tol = SNAP_PX / viewRef.current.scale;
      const snapped =
        g.kind === "move"
          ? snapMove(g.box, g.start, p, g.targets, tol, lastMods.current.shift)
          : snapScale(g.box, g.kind, g.start, p, g.targets, tol);
      point = snapped.point;
      guides = snapped.guides;
    }
    const m = gestureMatrix(g.kind, g.box, g.start, point, lastMods.current);
    // Only guides for lines the box really landed on: the matrix can discard part of a snap.
    const landed = boxBounds({ ...g.box, frame: compose(g.box.frame, m) });
    gesture.current = { ...g, m, guides: keepLanded(guides, landed) };
  }, []);

  // Declared before App's draw effect runs, so the repaint that follows a view change already
  // draws the re-derived gesture.
  useEffect(() => {
    follow();
  }, [view, follow]);

  // ponytail: devicePixelRatio is read on resize only, so dragging the window from a Retina panel
  // to a 1x one keeps the old backing store until the next resize. Upgrade: a
  // matchMedia(`(resolution: ${dpr}dppx)`) listener that re-runs resize().
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      rendererRef.current?.resize(width, height, window.devicePixelRatio || 1);
      setSize({ w: width, h: height });
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [canvasRef, rendererRef]);

  // Fit the mat on first layout, when the mat changes (machine switch, a different preset) and on
  // request. The key is the mat's numbers, not the object: a snapshot rebuilds it on every edit,
  // and an edit is not a reason to throw away the operator's zoom. Resizing is not in the key, so
  // a window resize does not refit; the size is a dependency only so the first non-zero size does.
  const matKey = mat ? `${mat.x},${mat.y},${mat.w},${mat.h}` : null;
  const insetKey = `${inset.left},${inset.top},${inset.right},${inset.bottom}`;
  const fittedFor = useRef<string | null>(null);
  const lastMatKey = useRef<string | null>(null);
  useEffect(() => {
    const matNow = matRef.current;
    if (!matNow || matKey === null || size.w === 0 || size.h === 0) return;
    const matChanged = matKey !== lastMatKey.current;
    lastMatKey.current = matKey;
    // A machine change has to show the new mat even when the last request was Fit content.
    // Recording that as a mat request keeps the follow-up render from immediately fitting content
    // again: its key matches the mat fit this run already stored.
    if (matChanged && fitReq.kind !== "mat") setFitReq({ kind: "mat", n: fitReq.n });
    const kind = matChanged ? "mat" : fitReq.kind;
    const key = `${kind}#${fitReq.n}#${matKey}#${insetKey}`;
    if (fittedFor.current === key) return;
    fittedFor.current = key;
    const content = contentRef.current;
    const useContent = kind === "content" && content !== null && content.w > 0 && content.h > 0;
    setView(fitView(useContent ? content : matNow, size, useContent ? CONTENT_FIT_MARGIN_PX : FIT_MARGIN_PX, insetRef.current));
  }, [fitReq, matKey, insetKey, size.w, size.h, setView]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      // Registered natively because React's wheel listener is passive; without preventDefault the
      // webview scrolls or page-zooms instead of the canvas.
      e.preventDefault();
      const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? LINE_PX : 1;
      const screen = toCanvas(canvas, e.clientX, e.clientY);
      setCursorScreen(screen);
      lastScreen.current = screen;
      // WebKit reports a trackpad pinch as a wheel with ctrlKey set, so one branch serves both.
      if (e.ctrlKey || e.metaKey) zoomBy(screen, wheelFactor(e.deltaY * unit));
      else setView(panBy(viewRef.current, -e.deltaX * unit, -e.deltaY * unit));
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [canvasRef, setView, zoomBy]);

  useEffect(() => {
    // Keys typed into a field belong to the field. A focused button is different: clicking a tool
    // leaves focus on it, so ignoring buttons outright killed Space-pan after any toolbar click
    // (pr-test-analyzer on #298). Space over the canvas pans, and its keyup is swallowed so the
    // button it would have pressed stays unpressed.
    const onDown = (e: KeyboardEvent) => {
      if (isTyping(e.target)) return;
      if (e.code === "Space") {
        const onButton = e.target instanceof HTMLElement && e.target.tagName === "BUTTON";
        if (onButton && !overCanvas.current) return;
        spaceHeld.current = true;
        e.preventDefault();
        return;
      }
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "0") requestFit();
      else if (e.key === "1") zoomBy(centre(), CSS_PX_PER_MM / viewRef.current.scale);
      else if (e.key === "=" || e.key === "+") zoomBy(centre(), ZOOM_STEP);
      else if (e.key === "-") zoomBy(centre(), 1 / ZOOM_STEP);
      else return;
      // The webview has its own page zoom on these chords; the canvas zoom replaces it.
      e.preventDefault();
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code !== "Space") return;
      // A button activates on Space's keyup, not its keydown.
      if (spaceHeld.current) e.preventDefault();
      spaceHeld.current = false;
    };
    // A Space released while another window had focus never arrives here.
    const onBlur = () => {
      spaceHeld.current = false;
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
      window.removeEventListener("blur", onBlur);
    };
  }, [centre, requestFit, zoomBy]);

  // Every transform reaches the backend through here, so all producers share the one-in-flight
  // rule and the pending preview that stands in for an unread commit. An id listed twice moves by
  // both, in the preview as in the backend; a node under another listed node moves only with that
  // node in the backend but by both here, so the moves must not nest. Every caller sends one move
  // or align's units, which `outermost` keeps disjoint. Filtered here as well as by the producers
  // that decide whether to send, so a drag's release cannot put a non-finite matrix on the wire
  // either (type-design review on #301).
  function send(unfiltered: Move[], requestedAt: number) {
    const moves = effective(unfiltered);
    if (moves.length === 0) {
      repaint();
      gestureEnded();
      return;
    }
    const { expand } = latest.current;
    // What a refusal puts back. Not always nothing: a commit that landed but could not be re-read
    // leaves its preview standing for the geometry the backend holds, and this one was built on it.
    const base = pending.current;
    const mine: PendingPreview = {
      preview: applyMoves(current().scene, moves.map((mv) => ({ shapes: expand(mv.ids), m: mv.m }))),
      retireAt: Infinity,
    };
    pending.current = mine;
    inFlight.current = true;
    previewChanged();
    repaint();
    settling.current = latest.current.commit(moves, requestedAt).then((outcome) => {
      inFlight.current = false;
      // Applied: the preview is what the backend now holds, so it stays until the snapshot that
      // includes this commit has rendered. If that refresh failed, the next successful one will.
      // Refused: back to what stood before it, which retires by revision like any preview.
      if (outcome.kind === "refused") {
        if (pending.current === mine) pending.current = base;
      } else if (pending.current) {
        pending.current = { ...pending.current, retireAt: outcome.snapshotRev ?? latest.current.sceneRev + 1 };
      }
      // Drained after the render this settles into, not here: the commit's refresh has set a new
      // document that has not rendered yet, and a queued edit built now would read the old tree
      // and scene (Copilot on #301). The effect below runs once React has rendered it. With nothing
      // queued there is nothing to wait for, and staying busy only swallowed the next press.
      drainDue.current = queued.current.size > 0;
      previewChanged();
      repaint();
    });
  }

  async function replaceDocument(load: () => Promise<unknown>) {
    // One at a time. A second, overlapping replacement cleared the shared flag when the first ended
    // while it was still loading, and old-document edits could reach it (Copilot on #301). Refused
    // before anything changes, so run() shows it. A load after this one has returned is fine: App
    // drops a snapshot asked for before it, so the first load's cannot lift `stale()` over it.
    if (replacing.current) throw new Error("another document is still loading");
    replacing.current = true;
    // Rendered now, so the panel locks for the load (`editsLock`) rather than when it ends.
    previewChanged();
    // A drag still under the pointer is not a commit yet, so nothing below waits for it, and its
    // pointer-up would send the old ids and matrix into the loaded document (Copilot on #301).
    // Dropped here, before the first await, so its release has nothing to send.
    gesture.current = null;
    repaint();
    let replaced = false;
    try {
      // A commit already on the wire settles first. Today the backend runs these sync commands in
      // the order they were sent, but that is Tauri's scheduling, not a promise of ours: made async,
      // a load could land first and the commit's ids would meet the new document (CodeRabbit on #301).
      await settling.current;
      await load();
      replaced = true;
      loadedAtRev.current = latest.current.sceneRev;
      // The old document's preview has nothing left to stand in for.
      pending.current = null;
    } finally {
      replacing.current = false;
      // Before anything can drain: every queued edit names ids from the document that just left.
      if (replaced) queued.current.clear();
      drainDue.current = queued.current.size > 0;
      previewChanged();
    }
  }

  // Moves that would change nothing are dropped, so a no-op never holds the wire. So is a move that
  // is not a finite number, from any producer: it crosses IPC as null and comes back as an error
  // that names nothing the operator did (a field typed as 1e400 is Infinity).
  const effective = (moves: Move[]) => moves.filter((mv) => mv.m.every(Number.isFinite) && !isIdentity(mv.m));

  // Busy from send until the queue has drained after that commit settles. inFlight alone cleared on
  // settle, a render before the drain, and an edit or drag in that gap went out at once and
  // overlapped the drained one on the wire (CodeRabbit on #301). And while a load replaces the
  // document, which nothing should edit until it is there.
  // A transform drag under the pointer counts too: its release sends a commit, so an align pressed
  // from the keyboard meanwhile went out first and the release put a second on the wire (Copilot
  // and CodeRabbit on #301). It waits in the queue, which `gestureEnded` drains.
  const busy = () =>
    inFlight.current || drainDue.current || replacing.current || stale() || gesture.current?.t === "transform";

  // A transform gesture ended without a commit (a no-op release, a cancel, nothing finite to send):
  // edits queued behind it have no settle to drain them, so drain on the next render instead.
  function gestureEnded() {
    if (queued.current.size === 0 || inFlight.current) return;
    drainDue.current = true;
    previewChanged();
  }

  function transformEach(key: string, make: (s: Scene, now: SendTime) => Move[]) {
    // Dropped, not queued: it was aimed at shapes of the document that just left.
    if (stale()) return;
    const at = performance.now();
    if (busy()) {
      queued.current.delete(key);
      queued.current.set(key, { make, at });
      return;
    }
    const moves = effective(make(current().scene, sendTime()));
    if (moves.length > 0) send(moves, at);
  }

  // Keyed by field and selection: a newer X edit replaces an older one, but an edit to Y does not
  // replace X, since each changes only its own axis (CodeRabbit and Copilot on #298).
  function transformWith(field: PropertyField, ids: number[], make: (s: Scene) => Matrix | null) {
    transformEach(`${field}:${ids.join(",")}`, (s) => {
      const m = make(s);
      return m ? [{ ids, m }] : [];
    });
  }

  // Sends queued edits in the order they were made, one per settled commit. An edit that comes out
  // as no change (its field already says that) is dropped so it cannot stall the rest.
  function drain() {
    // A send in flight drains on its own settle; draining now would put a second on the wire. A load
    // in progress decides whether the queue is still meant for this document.
    if (inFlight.current || replacing.current) return;
    for (const [key, { make, at }] of queued.current) {
      queued.current.delete(key);
      const moves = effective(make(current().scene, sendTime()));
      if (moves.length > 0) {
        send(moves, at);
        return;
      }
    }
  }

  const onPointerDown = (e: PointerEvent<HTMLCanvasElement>) => {
    const screen = toCanvas(e.currentTarget, e.clientX, e.clientY);
    lastScreen.current = screen;
    lastMods.current = { shift: e.shiftKey, alt: e.altKey };
    snapOff.current = e.metaKey || e.ctrlKey;
    const v = viewRef.current;
    const p = screenToWorld(v, screen);
    if (e.button === 1 || (e.button === 0 && spaceHeld.current)) {
      e.currentTarget.setPointerCapture(e.pointerId);
      gesture.current = { t: "pan", last: screen };
      return;
    }
    if (e.button !== 0) return;
    // Capture replaces SP3's window-level mouseup listener: a drag released outside the canvas
    // still ends here, so its preview is committed instead of stranded.
    e.currentTarget.setPointerCapture(e.pointerId);
    const { scene: s, selected: sel } = current();
    const shapes = latest.current.expand(sel);
    const box = selectionBox(s, shapes);
    const kind = box ? handleUnder(box, p, v.scale) : null;
    // Shift-click inside the box toggles the node under the pointer rather than dragging.
    if (box && kind && !(kind === "move" && e.shiftKey)) {
      if (busy()) return;
      const targets = snapTargets(s, shapes, latest.current.artboard);
      gesture.current = { t: "transform", kind, box, ids: sel, shapes, start: p, m: IDENTITY, targets, guides: [] };
      return;
    }
    const hit = hitTest(s, p.x, p.y, HIT_TOL_PX / v.scale);
    if (hit === null) {
      gesture.current = { t: "marquee", start: p, cur: p, additive: e.shiftKey };
      return;
    }
    const next = e.shiftKey ? toggleId(sel, hit) : [hit];
    setSelected(next);
    if (busy()) {
      gesture.current = null;
      return;
    }
    // Only drag when the hit node is in the new selection — a Shift-click that toggles a node
    // out must not start dragging the rest.
    const nextShapes = latest.current.expand(next);
    const nextBox = next.includes(hit) ? selectionBox(s, nextShapes) : null;
    gesture.current = nextBox
      ? {
          t: "transform", kind: "move", box: nextBox, ids: next, shapes: nextShapes, start: p, m: IDENTITY,
          targets: snapTargets(s, nextShapes, latest.current.artboard), guides: [],
        }
      : null;
  };

  const onPointerMove = (e: PointerEvent<HTMLCanvasElement>) => {
    const canvas = e.currentTarget;
    const screen = toCanvas(canvas, e.clientX, e.clientY);
    const v = viewRef.current;
    const p = screenToWorld(v, screen);
    setCursorScreen(screen);
    lastScreen.current = screen;
    lastMods.current = { shift: e.shiftKey, alt: e.altKey };
    snapOff.current = e.metaKey || e.ctrlKey;
    const g = gesture.current;
    if (!g) {
      const { scene: s, selected: sel } = current();
      const box = selectionBox(s, latest.current.expand(sel));
      const kind = spaceHeld.current ? "pan" : box ? handleUnder(box, p, v.scale) : null;
      // ponytail: resize cursors are screen-aligned, so on a rotated box they point along the
      // screen rather than the edge. Upgrade: choose by the handle's on-screen angle.
      canvas.style.cursor = kind ? CURSORS[kind] : "default";
      return;
    }
    if (g.t === "pan") {
      setView(panBy(v, screen.x - g.last.x, screen.y - g.last.y));
      gesture.current = { ...g, last: screen };
      return;
    }
    follow();
    repaint();
  };

  const onPointerUp = () => {
    const g = gesture.current;
    gesture.current = null;
    if (!g || g.t === "pan") return;
    if (g.t === "marquee") {
      const travel = Math.hypot(g.cur.x - g.start.x, g.cur.y - g.start.y) * viewRef.current.scale;
      const { scene: s, selected: sel } = current();
      if (travel < CLICK_SLOP_PX) {
        if (!g.additive) setSelected([]);
      } else {
        setSelected(marqueeSelection(sel, marqueeHits(s, normalizeRect(g.start, g.cur)), g.additive));
      }
      // The band is drawn imperatively, and a selection that did not change re-renders nothing.
      repaint();
      return;
    }
    if (isIdentity(g.m)) {
      repaint();
      gestureEnded();
      return;
    }
    send([{ ids: g.ids, m: g.m }], performance.now());
  };

  const onPointerCancel = () => {
    const wasTransform = gesture.current?.t === "transform";
    gesture.current = null;
    repaint();
    if (wasTransform) gestureEnded();
  };

  const onPointerEnter = () => {
    overCanvas.current = true;
  };

  const onPointerLeave = () => {
    overCanvas.current = false;
    setCursorScreen(null);
  };

  return {
    view,
    size,
    cursor: cursorScreen ? screenToWorld(view, cursorScreen) : null,
    requestFit,
    requestFitContent,
    zoomIn,
    zoomOut,
    repaint,
    editsLock: editsLockNow(),
    editsLockNow,
    effectiveScene: gestureScene(pending.current, scene, sceneRev),
    transformWith,
    transformEach,
    replaceDocument,
    handlers: { onPointerEnter, onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onPointerLeave },
  };
}
