// Drives a running Textchum with real input and checks that it behaves
// as INTERACTION.md says: who has the keyboard while a balloon is up,
// whether a press lands, what Escape and Return do, whether a picker
// remembers. The smoke tests call the code directly and cannot see
// any of this; it is only visible when the events come the way a
// person's do, through the window server.
//
//   osascript -l JavaScript scripts/feel.js
//
// scripts/feel.sh sets the app up on a fixture and runs this; it is
// not meant to be run on its own. The fixture is fixed: its first line
// starts with a four-letter word the fake server reports a finding
// on, and its fourth line is `    println!("{total}");`.
ObjC.import('AppKit');
ObjC.import('ApplicationServices');
ObjC.import('CoreGraphics');

const se = Application('System Events');
const proc = () => se.processes.byName('Textchum');
const results = [];

function pause(seconds) { delay(seconds); }

function post(type, point, clicks) {
  const event = $.CGEventCreateMouseEvent(null, type, point, 0);
  if (clicks) $.CGEventSetIntegerValueField(event, $.kCGMouseEventClickState, clicks);
  $.CGEventPost($.kCGHIDEventTap, event);
}
function move(point) { post($.kCGEventMouseMoved, point, 0); }
function click(point) {
  move(point); pause(0.05);
  post($.kCGEventLeftMouseDown, point, 1); post($.kCGEventLeftMouseUp, point, 1);
}
function doubleClick(point) {
  click(point); pause(0.08);
  post($.kCGEventLeftMouseDown, point, 2); post($.kCGEventLeftMouseUp, point, 2);
}
function key(code, modifiers) {
  if (modifiers) se.keyCode(code, { using: modifiers });
  else se.keyCode(code);
}
function type(text) { se.keystroke(text); }

// Every on-screen window of the app, for telling a balloon or a panel
// from the document's window.
function windows() {
  const list = ObjC.deepUnwrap(ObjC.castRefToObject(
    $.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements, 0))) || [];
  return list.filter((w) => w.kCGWindowOwnerName === 'Textchum');
}

// The document's window: the largest, since a balloon or a panel can
// be in front of it and be the first in the list.
function documentWindow() {
  let best = null;
  let bestArea = 0;
  for (const window of proc().windows()) {
    try {
      const [width, height] = window.size();
      if (width * height > bestArea) { best = window; bestArea = width * height; }
    } catch (_) {}
  }
  return best;
}

// Looked up afresh each time: an edit can leave a reference to the
// text area dangling, and a balloon has a text area of its own.
function textArea() {
  for (const element of documentWindow().entireContents()) {
    try { if (element.role() === 'AXTextArea') return element; } catch (_) {}
  }
  throw new Error('no text area');
}
// AXSelectedTextRange reads as [start, end], start counted from one,
// end the last selected character: an empty selection at offset n is
// [n + 1, n].
function selection(area) {
  const [start, end] = area.attributes.byName('AXSelectedTextRange').value();
  return { location: start - 1, length: end - start + 1 };
}
function focusedRole() {
  try { return proc().attributes.byName('AXFocusedUIElement').value().role(); } catch (_) { return null; }
}

// Rules the code is known to break today, by the issue that fixes it:
// reported, and not counted as a failure, until that lands.
const KNOWN = {};

function frontmostName() {
  try { return se.processes.whose({ frontmost: true })[0].name(); } catch (_) { return null; }
}

// Textchum in front, for certain: the first click on a window of an
// app that is not in front only brings it forward, and never reaches
// the text, so a driver that clicks into a background app checks
// nothing. Asked for, then clicked into by the title bar if asking was
// not enough.
function bringForward(window) {
  for (let attempt = 0; attempt < 4 && frontmostName() !== 'Textchum'; attempt++) {
    proc().frontmost = true;
    pause(0.6);
    if (frontmostName() === 'Textchum') break;
    const [x, y] = window.position();
    const [width] = window.size();
    click({ x: x + width / 2, y: y + 10 });
    pause(0.6);
  }
  return frontmostName() === 'Textchum';
}

function check(rule, what, ok, detail) {
  const mark = ok ? 'ok    ' : KNOWN[rule] ? 'known ' : 'FAIL  ';
  const why = ok ? '' : (detail ? ' — ' + detail : '') + (KNOWN[rule] ? ' (' + KNOWN[rule] + ')' : '');
  results.push(mark + rule.padEnd(5) + what + why);
}

let step = 'setup';

function run() {
  try {
    drive();
  } catch (error) {
    results.push('FAIL  —    the driver stopped at "' + step + '": ' + error);
  }
  return results.join('\n');
}

function drive() {
  // Posting mouse events needs this process to be trusted, and an
  // untrusted one has them dropped without a word.
  if (!$.AXIsProcessTrusted()) {
    results.push('FAIL  —    this terminal may not control the computer: allow it in System Settings ▸ Privacy & Security ▸ Accessibility');
    return;
  }
  const app = proc();
  const win = documentWindow();
  win.position = [80, 60];
  win.size = [1000, 620];
  pause(1);
  if (!bringForward(win)) {
    results.push('FAIL  —    Textchum would not come to the front (' + frontmostName() + ' is): nothing can be checked');
    return;
  }

  step = 'layout';
  // Where a character is on screen, from the font the profile sets and
  // the text area's frame; checked against where a click lands.
  const font = $.NSFont.fontWithNameSize('Menlo', 13);
  const attributes = $.NSDictionary.dictionaryWithObjectForKey(font, $.NSFontAttributeName);
  const charWidth = $.NSString.alloc.initWithUTF8String('M').sizeWithAttributes(attributes).width;
  const lineHeight = $.NSLayoutManager.alloc.init.defaultLineHeightForFont(font);
  const [ax, ay] = textArea().position();
  const at = (line, column) => ({ x: ax + 5 + (column + 0.5) * charWidth, y: ay + (line + 0.5) * lineHeight });
  const lineStart = (line) => {
    const text = textArea().value();
    let offset = 0;
    for (let n = 0; n < line; n++) offset = text.indexOf('\n', offset) + 1;
    return offset;
  };

  step = 'calibration';
  // Calibration: a click on the fourth line, sixth column, puts the
  // caret there. If it does not, nothing after can be trusted.
  click(at(3, 6)); pause(0.5);
  const placed = selection(textArea());
  const calibrated = placed.length === 0 && placed.location === lineStart(3) + 6;
  check('—', 'a click lands where the driver aims', calibrated,
    'caret at ' + JSON.stringify(placed) + ', aimed at ' + (lineStart(3) + 6) +
    ' — text at ' + JSON.stringify(textArea().position()) + ', front: ' + frontmostName());
  if (!calibrated) return;

  step = '1.1';
  // 1.1 The keyboard stays in the text while a finding's balloon is up.
  const before = windows().length;
  move(at(0, 2)); pause(1.4);
  const balloonUp = windows().length > before;
  check('4.1', 'resting on a finding opens its balloon', balloonUp,
    windows().length + ' windows, ' + before + ' before');
  const kept = selection(textArea());
  check('1.1', 'the caret stays where it was while the balloon is up',
    kept.location === placed.location && kept.length === 0 && focusedRole() === 'AXTextArea',
    JSON.stringify(kept) + ' focus ' + focusedRole());
  type('Z'); pause(0.4);
  const typed = textArea().value().substr(lineStart(3) + 6, 1) === 'Z';
  check('1.1', 'a key typed then lands at the caret', typed,
    JSON.stringify(textArea().value().substr(lineStart(3), 12)));
  key(6, ['command down']); pause(0.4); // ⌘Z

  step = '1.4';
  // 1.4 A double-click on the word under a balloon selects the word.
  move(at(0, 2)); pause(1.4);
  doubleClick(at(0, 2)); pause(0.5);
  const word = selection(textArea());
  // The fixture's first word is `make_thing`: a name as the language
  // spells it, underscore and all (10.2).
  check('1.4', 'a double-click under a balloon selects the word',
    word.location === 0 && word.length === 10, JSON.stringify(word));

  step = '2.1';
  // 2.1 Escape puts the balloon away.
  click(at(3, 6)); pause(0.3);
  move(at(0, 2)); pause(1.4);
  const withBalloon = windows().length;
  key(53); pause(0.5);
  check('2.1', 'Escape puts the balloon away', windows().length < withBalloon,
    windows().length + ' windows, ' + withBalloon + ' with the balloon');
  move(at(5, 30)); pause(0.6);

  step = '6.1';
  // 6.1 A command with nothing to do says so by the caret, and the
  // hint goes at the next key.
  click(at(3, 6)); pause(0.3);
  const quiet = windows().length;
  app.menuBars[0].menuBarItems.byName('Edit').menus[0].menuItems.byName('Unfold All').click();
  pause(0.6);
  const hinted = windows().length > quiet && focusedRole() === 'AXTextArea';
  check('6.1', 'a command with nothing to do hints by the caret, keyboard kept', hinted,
    windows().length + ' windows, focus ' + focusedRole());
  key(124); pause(0.5); // →
  check('6.1', 'the hint goes at the next key', windows().length === quiet, windows().length + ' windows');

  step = '3.1';
  // 3.1 A list opens on its first row, and Return takes it with
  // nothing typed: the diagnostics list, whose first row is line one.
  click(at(3, 6)); pause(0.3);
  key(14, ['command down', 'shift down']); pause(1); // ⇧⌘E
  key(36); pause(0.8); // ⏎
  const jumped = selection(textArea());
  check('3.1', 'Return in a list with nothing typed takes the first row',
    jumped.location === 0 && focusedRole() === 'AXTextArea', JSON.stringify(jumped) + ' focus ' + focusedRole());

  step = '13.1';
  // 13.1 The palette opens with what was typed into it last, selected.
  key(35, ['command down', 'shift down']); pause(0.8); // ⇧⌘P
  type('unfold'); pause(0.3);
  key(53); pause(0.5);
  key(35, ['command down', 'shift down']); pause(0.8);
  let field = null;
  try { field = app.attributes.byName('AXFocusedUIElement').value(); } catch (_) {}
  const value = field ? field.value() : null;
  const selected = field ? field.attributes.byName('AXSelectedText').value() : null;
  check('13.1', 'the palette opens with its last query, selected',
    value === 'unfold' && selected === 'unfold', JSON.stringify({ value: value, selected: selected }));
  key(53); pause(0.4);
}
