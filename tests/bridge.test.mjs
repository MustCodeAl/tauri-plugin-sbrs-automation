// Tests of src/bridge.js, the script the plugin injects into the app's page.
//
// The script runs here in a fake page: a window, a document and elements with
// fixed boxes, enough DOM for what the script touches. So these tests check
// the script's own logic (how it answers dialogs, turns actions into events,
// scrolls and measures elements), not how a real webview behaves; the
// end-to-end test of seleniumbase-rs does that on a real app.
//
// Run them with `node --test tests/bridge.test.mjs`; `cargo test` runs them
// too (tests/bridge_js.rs).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const SOURCE = readFileSync(new URL('../src/bridge.js', import.meta.url), 'utf8');
const KEY = 'element-6066-11e4-a52e-4f735466cecf';
const BACKSPACE = '';

class Event {
  constructor(type, init = {}) {
    Object.assign(this, init);
    this.type = type;
    this.defaultPrevented = false;
  }

  preventDefault() {
    if (this.cancelable) this.defaultPrevented = true;
  }
}
class MouseEvent extends Event {}
class PointerEvent extends MouseEvent {}
class KeyboardEvent extends Event {}

class Element {
  constructor(page, tag, { id = '', box = null, parent = null, focusable = false } = {}) {
    this.page = page;
    this.nodeType = 1;
    this.localName = tag;
    this.tagName = tag.toUpperCase();
    this.id = id;
    // Where the element is in the document, before any scrolling.
    this.box = box || { left: 0, top: 0, width: 0, height: 0 };
    this.parentElement = parent;
    this.focusable = focusable;
    this.isConnected = true;
    this.isContentEditable = false;
    this.disabled = false;
    this.readOnly = false;
  }

  get ownerDocument() {
    return this.page.document;
  }

  getBoundingClientRect() {
    const { left, top, width, height } = this.box;
    const x = left - this.page.scrollX;
    const y = top - this.page.scrollY;
    return { x, y, left: x, top: y, right: x + width, bottom: y + height, width, height };
  }

  scrollIntoView(options) {
    this.page.scrollTo(this, options);
  }

  contains(other) {
    for (let node = other; node; node = node.parentElement) if (node === this) return true;
    return false;
  }

  closest() {
    // Only asked for focusable elements.
    for (let node = this; node; node = node.parentElement) if (node.focusable) return node;
    return null;
  }

  focus() {
    this.page.document.activeElement = this;
  }

  blur() {
    if (this.page.document.activeElement === this) this.page.document.activeElement = this.page.document.body;
  }

  click() {
    this.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 }));
  }

  dispatchEvent(event) {
    this.page.events.push({
      type: event.type, target: this.id, button: event.button, detail: event.detail, x: event.clientX, y: event.clientY,
    });
    return !event.defaultPrevented;
  }
}
class HTMLInputElement extends Element {
  constructor(page, options) {
    super(page, 'input', options);
    this.type = 'text';
    this.form = null;
    this.text = '';
  }

  get value() {
    return this.text;
  }

  set value(value) {
    this.text = String(value);
  }
}
class HTMLTextAreaElement extends Element {}

// A page of 800 x 600 CSS pixels whose document is `height` tall. With
// `smooth`, it has `scroll-behavior: smooth`: a scroll that does not ask for
// `instant` moves the page over the frames that follow, after the script has
// run, so the script still sees the old place.
class Page {
  constructor({ height = 600, smooth = false } = {}) {
    this.height = height;
    this.smooth = smooth;
    this.scrollX = 0;
    this.scrollY = 0;
    this.now = 1000;
    this.events = [];
    this.scrolls = [];
    this.replies = [];
    this.calls = 0;
    this.elements = [];
    const page = this;
    const html = new Element(this, 'html', { box: { left: 0, top: 0, width: 800, height } });
    const body = new Element(this, 'body', { box: { left: 0, top: 0, width: 800, height }, parent: html });
    this.document = {
      documentElement: html,
      body,
      activeElement: body,
      readyState: 'complete',
      title: 'Fake',
      get defaultView() {
        return page.window;
      },
      querySelector: (selector) => this.elements.find((element) => `#${element.id}` === selector) || null,
      querySelectorAll: (selector) => this.elements.filter((element) => `#${element.id}` === selector),
      getElementsByTagName: (tag) => this.elements.filter((element) => element.localName === tag),
      elementFromPoint: (x, y) => this.at(x, y),
      // The fake page does no editing: the script's own fallback edits.
      execCommand: () => false,
    };
    this.window = {
      document: this.document,
      length: 0,
      innerWidth: 800,
      innerHeight: 600,
      get scrollX() {
        return page.scrollX;
      },
      get scrollY() {
        return page.scrollY;
      },
      Event,
      MouseEvent,
      PointerEvent,
      KeyboardEvent,
      HTMLInputElement,
      HTMLTextAreaElement,
      Function,
      getComputedStyle: (element) => ({
        display: 'block',
        visibility: 'visible',
        opacity: '1',
        paddingLeft: `${element.padding || 0}px`,
        paddingTop: `${element.padding || 0}px`,
        getPropertyValue: () => '',
      }),
      __TAURI_INTERNALS__: {
        invoke: async (command, payload) => {
          assert.equal(command, 'plugin:sbrs-automation|reply');
          this.replies.push(payload);
        },
      },
    };
    this.window.window = this.window;
    this.window.top = this.window;
    const globals = {
      window: this.window,
      document: this.document,
      addEventListener: () => {},
      location: { href: 'tauri://localhost/' },
      history: {},
      performance: { now: () => this.now },
      setTimeout: () => 0,
      XPathResult: {},
    };
    new Function(...Object.keys(globals), SOURCE)(...Object.values(globals));
  }

  // Adds an element, a child of `parent` (else of the body).
  add(tag, { parent, ...options } = {}) {
    const holder = parent || this.document.body;
    const element = tag === 'input'
      ? new HTMLInputElement(this, { ...options, parent: holder, focusable: true })
      : new Element(this, tag, { ...options, parent: holder });
    this.elements.push(element);
    return element;
  }

  // The innermost element at a point of the viewport.
  at(x, y) {
    const left = x + this.scrollX;
    const top = y + this.scrollY;
    for (const element of [...this.elements].reverse()) {
      const { box } = element;
      if (left >= box.left && left < box.left + box.width && top >= box.top && top < box.top + box.height) {
        return element;
      }
    }
    return null;
  }

  scrollTo(element, options = {}) {
    const behavior = options.behavior || 'auto';
    this.scrolls.push(behavior);
    if (behavior === 'smooth' || (behavior === 'auto' && this.smooth)) return;
    const { box } = element;
    this.scrollY = Math.min(Math.max(0, box.top + box.height / 2 - 300), this.height - 600);
  }

  // Runs one operation of the script as the plugin does, and returns its
  // value or throws its WebDriver error.
  async run(op, args) {
    this.calls += 1;
    const id = this.calls;
    await this.window.__sbrs.run(id, op, args);
    const reply = this.replies.find((candidate) => candidate.id === id);
    assert.ok(reply, `no reply to ${op}`);
    if (!reply.ok) {
      const error = new Error(reply.value.message);
      error.code = reply.value.error;
      throw error;
    }
    return reply.value;
  }

  // The script's reference to an element.
  async ref(element) {
    const found = await this.run('find', { using: 'css selector', value: `#${element.id}`, all: false });
    return found[KEY];
  }

  dialog(action, text) {
    return this.run('dialog', { action, text });
  }

  input(...actions) {
    return this.run('input', { actions: actions.map((action) => ({ source: 'mouse', ...action })) });
  }

  // The events dispatched of the given types.
  seen(...types) {
    return this.events.filter((event) => types.includes(event.type));
  }
}

// Expects `promise` to fail with the WebDriver error `code`.
const refused = async (promise, code) => {
  await assert.rejects(promise, (error) => {
    assert.equal(error.code, code, error.message);
    return true;
  });
};

test('with no answer set, an alert is accepted and a confirm and a prompt are dismissed', async () => {
  const page = new Page();
  assert.equal(page.window.alert('Saved'), undefined);
  assert.equal(await page.dialog('text'), 'Saved');
  await page.dialog('accept');
  assert.equal(page.window.confirm('Delete?'), false);
  assert.equal(await page.dialog('text'), 'Delete?');
  await page.dialog('dismiss');
  assert.equal(page.window.prompt('Name?', 'nobody'), null);
  await page.dialog('dismiss');
  await refused(page.dialog('text'), 'no such alert');
});

test('the answer set before a dialog opens is what the page gets', async () => {
  const page = new Page();
  await page.dialog('accept');
  assert.equal(page.window.confirm('Delete?'), true);
  await page.dialog('accept');
  await page.dialog('type', 'Ada');
  await page.dialog('accept');
  assert.equal(page.window.prompt('Name?', 'nobody'), 'Ada');
  await page.dialog('accept');
  await page.dialog('accept');
  assert.equal(page.window.prompt('Name?', 'nobody'), 'nobody');
});

test('a right click sends auxclick and contextmenu, and no click', async () => {
  const page = new Page();
  const menu = page.add('div', { id: 'menu', box: { left: 100, top: 100, width: 50, height: 20 } });
  await page.input(
    { type: 'pointerMove', origin: { element: await page.ref(menu) }, x: 0, y: 0 },
    { type: 'pointerDown', button: 2 },
    { type: 'pointerUp', button: 2 },
  );
  const seen = page.seen('mousedown', 'mouseup', 'click', 'auxclick', 'contextmenu');
  assert.deepEqual(seen.map(({ type, target, button, x, y }) => [type, target, button, x, y]), [
    ['mousedown', 'menu', 2, 125, 110],
    ['mouseup', 'menu', 2, 125, 110],
    ['auxclick', 'menu', 2, 125, 110],
    ['contextmenu', 'menu', 2, 125, 110],
  ]);
});

test('a drag from one element to another clicks the element that holds both', async () => {
  const page = new Page();
  const list = page.add('ul', { id: 'list', box: { left: 0, top: 100, width: 400, height: 200 } });
  const from = page.add('li', { id: 'from', parent: list, box: { left: 0, top: 100, width: 400, height: 50 } });
  const to = page.add('li', { id: 'to', parent: list, box: { left: 0, top: 200, width: 400, height: 50 } });
  await page.input(
    { type: 'pointerMove', origin: { element: await page.ref(from) }, x: 0, y: 0 },
    { type: 'pointerDown', button: 0 },
    { type: 'pointerMove', origin: { element: await page.ref(to) }, x: 0, y: 0 },
    { type: 'pointerUp', button: 0 },
  );
  const seen = page.seen('mousedown', 'mousemove', 'mouseup', 'click', 'dblclick');
  assert.deepEqual(seen.map(({ type, target, detail }) => [type, target, detail]), [
    ['mousemove', 'from', undefined],
    ['mousedown', 'from', 1],
    ['mousemove', 'to', undefined],
    ['mouseup', 'to', 1],
    ['click', 'list', 1],
  ]);
});

test('Backspace deletes the last character of the focused field, and the page sees input', async () => {
  const page = new Page();
  const field = page.add('input', { id: 'name', box: { left: 0, top: 0, width: 200, height: 20 } });
  field.value = 'Adam';
  field.focus();
  await page.run('input', {
    actions: [
      { source: 'keyboard', type: 'keyDown', value: BACKSPACE },
      { source: 'keyboard', type: 'keyUp', value: BACKSPACE },
    ],
  });
  assert.equal(field.value, 'Ada');
  const seen = page.seen('keydown', 'keypress', 'input', 'keyup');
  assert.deepEqual(seen.map(({ type, target }) => [type, target]), [
    ['keydown', 'name'],
    ['input', 'name'],
    ['keyup', 'name'],
  ]);
});

test('accepting a confirm that was dismissed when it opened is refused; dismissing it clears it', async () => {
  const page = new Page();
  assert.equal(page.window.confirm('Delete?'), false);
  await refused(page.dialog('accept'), 'unsupported operation');
  assert.equal(await page.dialog('text'), 'Delete?');
  await page.dialog('dismiss');
  await refused(page.dialog('text'), 'no such alert');
});

test('dismissing a confirm that was accepted when it opened is refused; accepting it clears it', async () => {
  const page = new Page();
  await page.dialog('accept');
  assert.equal(page.window.confirm('Delete?'), true);
  await refused(page.dialog('dismiss'), 'unsupported operation');
  await page.dialog('accept');
  await refused(page.dialog('text'), 'no such alert');
});

test('a prompt that opened is accepted or dismissed only as it was answered', async () => {
  const page = new Page();
  assert.equal(page.window.prompt('Name?'), null);
  await refused(page.dialog('accept'), 'unsupported operation');
  await page.dialog('dismiss');
  await page.dialog('accept');
  assert.equal(page.window.prompt('Name?', 'nobody'), 'nobody');
  await refused(page.dialog('dismiss'), 'unsupported operation');
  await page.dialog('accept');
  await refused(page.dialog('text'), 'no such alert');
});

test('an alert that opened may be accepted or dismissed', async () => {
  const page = new Page();
  page.window.alert('Saved');
  await page.dialog('dismiss');
  page.window.alert('Saved again');
  await page.dialog('accept');
  await refused(page.dialog('text'), 'no such alert');
});

test('an alert leaves the answer set for the confirm that follows it', async () => {
  const page = new Page();
  await page.dialog('accept');
  page.window.alert('Unsaved changes');
  assert.equal(page.window.confirm('Leave?'), true);
});

test('a second dialog does not hide the first: they are read and cleared oldest first', async () => {
  const page = new Page();
  page.window.alert('Unsaved changes');
  page.window.confirm('Leave?');
  assert.equal(await page.dialog('text'), 'Unsaved changes');
  await page.dialog('accept');
  assert.equal(await page.dialog('text'), 'Leave?');
  await refused(page.dialog('accept'), 'unsupported operation');
  await page.dialog('dismiss');
  await refused(page.dialog('text'), 'no such alert');
});

test('only the last answer set counts, and the first confirm or prompt uses it up', async () => {
  const page = new Page();
  await page.dialog('accept');
  await page.dialog('dismiss');
  assert.equal(page.window.confirm('One?'), false);
  await page.dialog('dismiss');
  await page.dialog('accept');
  assert.equal(page.window.confirm('Two?'), true);
  assert.equal(page.window.prompt('Three?', 'nobody'), null);
});

test('a new session forgets the dialogs and the answer set', async () => {
  const page = new Page();
  page.window.alert('Saved');
  await page.dialog('accept');
  await page.dialog('accept');
  await page.run('reset');
  await refused(page.dialog('text'), 'no such alert');
  assert.equal(page.window.confirm('Delete?'), false);
});

test('a page that opens dialogs without end keeps only the last hundred recorded', async () => {
  const page = new Page();
  for (let number = 1; number <= 150; number += 1) page.window.alert(`Alert ${number}`);
  assert.equal(await page.dialog('text'), 'Alert 51');
});

// A page 3000 pixels tall that scrolls smoothly, and a button far below the
// part in view.
const smoothPage = () => {
  const page = new Page({ height: 3000, smooth: true });
  const far = page.add('button', { id: 'far', box: { left: 100, top: 2000, width: 200, height: 40 } });
  return { page, far };
};

test('an element screenshot measures the element once it is in view, on a page that scrolls smoothly', async () => {
  const { page, far } = smoothPage();
  const area = await page.run('prop', { id: await page.ref(far), what: 'viewrect' });
  assert.deepEqual(area, { x: 100, y: 280, width: 200, height: 40 });
  assert.deepEqual(page.scrolls, ['instant']);
});

test('a pointer move onto an element out of view lands on it, on a page that scrolls smoothly', async () => {
  const { page, far } = smoothPage();
  await page.input({ type: 'pointerMove', origin: { element: await page.ref(far) }, x: 0, y: 0 });
  assert.deepEqual(page.seen('mousemove').map(({ target, x, y }) => [target, x, y]), [['far', 200, 300]]);
  assert.deepEqual(page.scrolls, ['instant']);
});

test('a click scrolls the element into view at once, also on a page that scrolls smoothly', async () => {
  const { page, far } = smoothPage();
  await page.run('click', { id: await page.ref(far) });
  assert.deepEqual(page.seen('click').map(({ target }) => target), ['far']);
  assert.deepEqual(page.scrolls, ['instant']);
});

test('an element screenshot in a frame with a border and padding is cut where the element shows', async () => {
  const page = new Page();
  const frame = page.add('iframe', { id: 'frame', box: { left: 50, top: 50, width: 324, height: 224 } });
  frame.clientLeft = 2;
  frame.clientTop = 2;
  frame.padding = 10;
  // The frame's own page, 300 x 200 inside the frame's border and padding.
  const inner = { scrollX: 0, scrollY: 0, scrollTo: () => {}, events: [] };
  inner.window = {
    innerWidth: 300, innerHeight: 200, frameElement: frame, parent: page.window, getComputedStyle: page.window.getComputedStyle,
  };
  inner.document = { defaultView: inner.window };
  page.window.inside = new Element(inner, 'button', { id: 'inside', box: { left: 20, top: 30, width: 40, height: 10 } });
  const inside = await page.run('script', { script: 'return this.inside;', args: [], isAsync: false });
  const area = await page.run('prop', { id: inside[KEY], what: 'viewrect' });
  assert.deepEqual(area, { x: 82, y: 92, width: 40, height: 10 });
});
