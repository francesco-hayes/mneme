// What jsdom lacks and CodeMirror calls. Imported by setup.ts.
//
// CodeMirror's view is not written for jsdom: there is no layout to measure.
// The usual way to run it there anyway is to stub the few geometry and
// selection methods it reaches for, so that they answer "nothing" instead of
// throwing. Tests then drive the editor through its own API (see editor.ts).
// Each stub is added only if the method is missing, so a jsdom that gains one
// is used as it is.

const NO_RECTS = Object.assign([], { item: () => null });
const EMPTY_RECT = {
  x: 0,
  y: 0,
  top: 0,
  right: 0,
  bottom: 0,
  left: 0,
  width: 0,
  height: 0,
  toJSON: () => ({}),
};

// Measuring text. With no rectangles CodeMirror falls back to default sizes.
Range.prototype.getClientRects ??= () => NO_RECTS as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => EMPTY_RECT;

// Reading the selection inside a shadow root. jsdom says its vendor is Apple,
// so CodeMirror takes the path it uses in Safari, which is also the engine
// the real app runs in.
Selection.prototype.getComposedRanges ??= function (this: Selection) {
  return this.rangeCount > 0 ? [this.getRangeAt(0)] : [];
};
// What it tries next when that finds no selection (an old Safari workaround).
Document.prototype.execCommand ??= () => false;
