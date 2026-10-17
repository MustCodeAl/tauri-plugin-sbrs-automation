// Injected into every page of the app by tauri-plugin-sbrs-automation. Runs one
// WebDriver operation at a time for the plugin and sends the result back over
// Tauri's IPC (the plugin's `reply` command).
//
// This is the source. `src/bridge.js` is generated from it (`bun run bridge`)
// and is what the plugin embeds, so apps build without Node or Bun.
//
// It runs in the top page only and drives same-origin frames from there:
// commands act on the frame entered last. Dialogs (alert, confirm, prompt) and
// input (W3C actions) are simulated with page script and DOM events; the OS's
// mouse and keyboard are never used.
(() => {
  // The window, with the entry point this script adds and Tauri's IPC.
  type Page = Window & {
    __sbrs?: unknown;
    __TAURI_INTERNALS__: { invoke: (command: string, payload: unknown) => Promise<unknown> };
  };
  // A window with its constructors (a frame's own realm has its own).
  type View = Window & typeof globalThis;
  // An element, with the members only some kinds have.
  type El = HTMLElement & {
    disabled?: boolean;
    checked?: boolean;
    selected?: boolean;
    value?: string;
    readOnly?: boolean;
    href?: string;
    type?: string;
    form?: HTMLFormElement | null;
  };
  type Obj = Record<string, unknown>;
  type BridgeError = Error & { code?: string };
  // A reference to an element, as W3C WebDriver sends it.
  type ElementRef = { 'element-6066-11e4-a52e-4f735466cecf': string };
  type Context = { win: View; doc: Document };
  type Rect = { x: number; y: number; width: number; height: number };
  type Answer = { accept: boolean; text: string | null };
  type Dialog = { type: string; text: string; accepted: boolean };
  type Pointer = {
    x: number;
    y: number;
    over: El | null;
    buttons: Set<number>;
    down: { button: number; target: El; count: number } | null;
  };
  type Press = { button: number; target: El; time: number; count: number };
  type Origin = 'viewport' | 'pointer' | { element: string };

  // The arguments of each operation, as the plugin sends them.
  type Action =
    | { type: 'pointerMove'; source: string; origin: Origin; x: number; y: number }
    | { type: 'pointerDown' | 'pointerUp'; source: string; button: number }
    | { type: 'keyDown' | 'keyUp'; value: string };
  type Args = {
    info: { what: 'url' | 'title' | 'source' | 'ready' };
    go: { url: string };
    history: { action: 'back' | 'forward' | 'refresh' };
    find: { using: string; value: string; root?: string; all?: boolean };
    active: Record<string, never>;
    prop: { id: string; what: string; name: string };
    click: { id: string };
    clear: { id: string };
    keys: { id: string; text: string };
    script: { script: string; args?: unknown[]; isAsync?: boolean };
    dialog: { action: 'text' | 'type' | 'accept' | 'dismiss'; text?: string };
    frame: { top?: boolean; parent?: boolean; index?: number; element?: string };
    input: { actions: Action[] };
    release: Record<string, never>;
    reset: Record<string, never>;
  };
  type Ops = { [K in keyof Args]: (args: Args[K]) => unknown };

  const page = window as unknown as Page;
  if (page.__sbrs || window !== window.top) return;
  const KEY = 'element-6066-11e4-a52e-4f735466cecf';
  const ENTER = '';
  const elements = new Map<string, El>();
  let next = 0;
  let leaving = false;
  // The frame elements entered, outermost first.
  let frames: HTMLIFrameElement[] = [];
  addEventListener('hashchange', () => { leaving = false; });
  addEventListener('popstate', () => { leaving = false; });

  function fail(code: string, message: string): never {
    const error: BridgeError = new Error(message);
    error.code = code;
    throw error;
  }
  // A frame's nodes belong to the frame's own realm, where `instanceof`
  // against this page's classes is false: test the shape instead.
  const isElement = (value: unknown): value is El => value !== null && typeof value === 'object'
    && (value as Obj).nodeType === 1 && typeof (value as Obj).tagName === 'string';
  const isList = (value: unknown): boolean => Array.isArray(value)
    || ['[object NodeList]', '[object HTMLCollection]'].includes(Object.prototype.toString.call(value));
  const isWindow = (value: unknown): boolean => value !== null && typeof value === 'object'
    && (value as Obj).window === value;
  // The window of an element: a frame's, or this page's.
  const viewOf = (element: Element): View => (element.ownerDocument?.defaultView || window) as View;
  const ref = (element: El): string => {
    for (const [id, known] of elements) if (known === element) return id;
    const id = `e${++next}-${Math.random().toString(36).slice(2, 10)}`;
    elements.set(id, element);
    return id;
  };
  const get = (id: string): El => {
    const element = elements.get(id);
    if (!element) fail('no such element', `no element ${id} in this page`);
    if (!element.isConnected) fail('stale element reference', `element ${id} is no longer in the page`);
    return element;
  };
  const wrap = (value: unknown): unknown => {
    if (isElement(value)) return { [KEY]: ref(value) };
    if (isList(value)) return Array.from(value as ArrayLike<unknown>, wrap);
    if (value && typeof value === 'object' && !isWindow(value)) {
      const out: Obj = {};
      for (const key of Object.keys(value)) out[key] = wrap((value as Obj)[key]);
      return out;
    }
    return value === undefined ? null : value;
  };
  const unwrap = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(unwrap);
    if (value && typeof value === 'object') {
      const object = value as Obj;
      if (typeof object[KEY] === 'string') return get(object[KEY]);
      const out: Obj = {};
      for (const key of Object.keys(object)) out[key] = unwrap(object[key]);
      return out;
    }
    return value;
  };
  const visible = (element: El): boolean => {
    const style = viewOf(element).getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  // React and similar frameworks watch the native value setter, so use it.
  const setNative = (element: El, value: string): void => {
    const view = viewOf(element);
    const proto = element instanceof view.HTMLTextAreaElement ? view.HTMLTextAreaElement.prototype
      : element instanceof view.HTMLInputElement ? view.HTMLInputElement.prototype : null;
    const setter = proto && Object.getOwnPropertyDescriptor(proto, 'value')?.set;
    if (setter) setter.call(element, value);
    else if (element.isContentEditable) element.textContent = value;
    else element.value = value;
  };
  const setValue = (element: El, value: string): void => {
    setNative(element, value);
    const view = viewOf(element);
    element.dispatchEvent(new view.Event('input', { bubbles: true }));
    element.dispatchEvent(new view.Event('change', { bubbles: true }));
  };
  function find(root: Document | El, using: string, value: string, all: boolean): ElementRef | ElementRef[] {
    let found: (Node | null)[] = [];
    try {
      switch (using) {
        case 'css selector':
          found = all ? [...root.querySelectorAll(value)] : [root.querySelector(value)];
          break;
        case 'tag name':
          found = [...root.getElementsByTagName(value)];
          break;
        case 'link text':
        case 'partial link text':
          found = [...root.querySelectorAll('a')].filter((a) => (using === 'link text'
            ? a.innerText.trim() === value : a.innerText.includes(value)));
          break;
        case 'xpath': {
          const result = ((root as El).ownerDocument || (root as Document))
            .evaluate(value, root, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
          for (let i = 0; i < result.snapshotLength; i += 1) found.push(result.snapshotItem(i));
          break;
        }
        default:
          fail('invalid argument', `unknown locator strategy \`${using}\``);
      }
    } catch (error) {
      const caught = error as Partial<BridgeError> | null;
      if (caught?.code) throw error;
      fail('invalid selector', String(caught?.message || error));
    }
    const elementsFound = found.filter(isElement);
    if (all) return elementsFound.map((node) => ({ [KEY]: ref(node) }));
    const [first] = elementsFound;
    if (!first) fail('no such element', `no element matches ${using} \`${value}\``);
    return { [KEY]: ref(first) };
  }
  const leave = (go: () => void): null => {
    leaving = true;
    frames = [];
    setTimeout(go, 0);
    return null;
  };

  // Frames. Only a same-origin frame can be entered: the page script cannot
  // reach into the document of another origin.
  const frameDocument = (frame: HTMLIFrameElement): Document | null => {
    try {
      return frame.contentDocument;
    } catch {
      return null;
    }
  };
  // The window and document commands act on: the frame entered last, or
  // this page.
  const context = (): Context => {
    const frame = frames.at(-1);
    if (!frame) return { win: window as View, doc: document };
    const doc = frame.isConnected ? frameDocument(frame) : null;
    if (!doc || !doc.defaultView) {
      fail('no such frame', 'the frame entered is gone or is no longer same-origin; switch to its parent or the top page');
    }
    return { win: doc.defaultView as View, doc };
  };
  // Scrolls an element to the middle of the view at once, as a driver does:
  // under the page's CSS `scroll-behavior: smooth` a plain scrollIntoView
  // moves it over the frames that follow, after it has been measured.
  const scrollToCentre = (element: El): void => {
    element.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  };
  // Where an element shows in the app's viewport, after it is scrolled into
  // view and cut to the viewports it is in: the area of its screenshot.
  const viewportRect = (element: El): Rect => {
    scrollToCentre(element);
    let { left, top, right, bottom } = element.getBoundingClientRect();
    const clip = (view: Window): void => {
      left = Math.max(left, 0);
      top = Math.max(top, 0);
      right = Math.min(right, view.innerWidth);
      bottom = Math.min(bottom, view.innerHeight);
    };
    for (let view: View = viewOf(element); view !== window && view.frameElement; view = view.parent as View) {
      clip(view);
      const frame = view.frameElement as HTMLElement;
      // The frame's view starts inside its border (client) and its padding.
      const box = frame.getBoundingClientRect();
      const style = viewOf(frame).getComputedStyle(frame);
      const dx = box.left + frame.clientLeft + (parseFloat(style.paddingLeft) || 0);
      const dy = box.top + frame.clientTop + (parseFloat(style.paddingTop) || 0);
      left += dx;
      right += dx;
      top += dy;
      bottom += dy;
    }
    clip(window);
    if (right <= left || bottom <= top) fail('unable to capture screen', 'the element has no area in view');
    return { x: left, y: top, width: right - left, height: bottom - top };
  };

  // User prompts. A dialog cannot wait for the client (alert, confirm and
  // prompt block the page), so it answers at once. A confirm or prompt takes
  // the answer the client set before it opened (accept or dismiss, and the
  // text for a prompt), which no other dialog then gets, else it is
  // dismissed; an alert needs no answer and leaves the one set for the next
  // confirm or prompt. Each dialog stays recorded, with the answer the page
  // got, until the client accepts or dismisses it as it was answered, the
  // oldest first.
  const dialogs: { open: Dialog[]; answer: Answer | null; text: string | null } = {
    open: [],
    answer: null,
    text: null,
  };
  // At most this many dialogs stay recorded; the oldest are forgotten.
  const MAX_DIALOGS = 100;
  const ask = (
    type: 'alert' | 'confirm' | 'prompt',
    message?: unknown,
    fallback?: unknown,
  ): boolean | string | null | undefined => {
    let answer: Answer = { accept: true, text: null };
    if (type !== 'alert') {
      answer = dialogs.answer || { accept: false, text: null };
      dialogs.answer = null;
    }
    dialogs.open.push({ type, text: message === undefined ? '' : String(message), accepted: answer.accept });
    if (dialogs.open.length > MAX_DIALOGS) dialogs.open.shift();
    if (type === 'alert') return undefined;
    if (type === 'confirm') return answer.accept;
    if (!answer.accept) return null;
    if (answer.text !== null) return answer.text;
    return fallback === undefined ? '' : String(fallback);
  };
  const HOOKED = Symbol('sbrs-automation dialogs');
  // Replaces the dialogs of a window: this page's or a same-origin frame's.
  const hookDialogs = (win: Window): void => {
    try {
      if ((win as unknown as Record<symbol, unknown>)[HOOKED]) return;
      Object.defineProperty(win, HOOKED, { value: true });
      win.alert = (message?: unknown): void => { ask('alert', message); };
      win.confirm = (message?: string): boolean => ask('confirm', message) as boolean;
      win.prompt = (message?: string, value?: string): string | null => ask('prompt', message, value) as string | null;
    } catch {
      // A cross-origin frame keeps its own dialogs.
    }
  };
  // Hooks the dialogs of the same-origin frames of a window, and of theirs.
  const hookFrames = (win: Window): void => {
    for (let index = 0; index < win.length; index += 1) {
      const child = win[index];
      let reachable = false;
      try {
        reachable = Boolean(child?.document);
      } catch {
        // Cross-origin.
      }
      if (child && reachable) {
        hookDialogs(child);
        hookFrames(child);
      }
    }
  };
  hookDialogs(window);

  // Input (W3C actions): one keyboard, and a mouse for each pointer source,
  // as DOM events at the pointer's place in the current frame. Untrusted
  // events do not apply CSS :hover and do not start HTML drag and drop.
  const input: { keys: Set<string>; pointers: Map<string, Pointer>; press: Press | null } = {
    keys: new Set(),
    pointers: new Map(),
    press: null,
  };
  // How soon a second press on the same element makes a double click.
  const DOUBLE_CLICK_MS = 500;
  const pointerOf = (source: string): Pointer => {
    let pointer = input.pointers.get(source);
    if (!pointer) {
      pointer = { x: 0, y: 0, over: null, buttons: new Set(), down: null };
      input.pointers.set(source, pointer);
    }
    return pointer;
  };
  const modifiers = () => ({
    shiftKey: input.keys.has('Shift'),
    ctrlKey: input.keys.has('Control'),
    altKey: input.keys.has('Alt'),
    metaKey: input.keys.has('Meta'),
  });
  // `MouseEvent.buttons` for the pressed W3C buttons (0 left, 1 middle,
  // 2 right, 3 back, 4 forward).
  const buttonMask = (pressed: Set<number>): number => [...pressed]
    .reduce((mask, button) => mask | ([1, 4, 2, 8, 16][button] ?? 0), 0);
  const NOT_BUBBLING = new Set(['mouseenter', 'mouseleave', 'pointerenter', 'pointerleave']);
  const mouseEvent = (target: El, type: string, pointer: Pointer, extra: Obj = {}): boolean => {
    const view = viewOf(target);
    const bubbles = !NOT_BUBBLING.has(type);
    const init = {
      bubbles,
      cancelable: bubbles,
      composed: true,
      view,
      clientX: pointer.x,
      clientY: pointer.y,
      screenX: pointer.x,
      screenY: pointer.y,
      button: 0,
      buttons: buttonMask(pointer.buttons),
      ...modifiers(),
      ...extra,
    } as PointerEventInit;
    if (type.startsWith('pointer') && view.PointerEvent) {
      return target.dispatchEvent(new view.PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true, ...init,
      }));
    }
    return target.dispatchEvent(new view.MouseEvent(type, init));
  };
  // The element under the pointer, in the current frame.
  const hit = (pointer: Pointer): El => {
    const { doc } = context();
    return (doc.elementFromPoint(pointer.x, pointer.y) || doc.documentElement) as El;
  };
  const movePointer = (pointer: Pointer, { origin, x, y }: { origin: Origin; x: number; y: number }): void => {
    const { win, doc } = context();
    let left = x;
    let top = y;
    if (origin === 'pointer') {
      left += pointer.x;
      top += pointer.y;
    } else if (origin !== 'viewport') {
      const element = get(origin.element);
      if (element.ownerDocument !== doc) fail('no such element', 'the element is not in the current frame');
      let rect = element.getBoundingClientRect();
      const centre = (): [number, number] => [rect.left + rect.width / 2, rect.top + rect.height / 2];
      const [cx, cy] = centre();
      if (cx < 0 || cy < 0 || cx > win.innerWidth || cy > win.innerHeight) {
        scrollToCentre(element);
        rect = element.getBoundingClientRect();
      }
      const [ex, ey] = centre();
      left += Math.floor(ex);
      top += Math.floor(ey);
    }
    if (left < 0 || top < 0 || left > win.innerWidth || top > win.innerHeight) {
      fail('move target out of bounds', `(${left}, ${top}) is outside the viewport, ${win.innerWidth} x ${win.innerHeight}`);
    }
    pointer.x = left;
    pointer.y = top;
    const target = hit(pointer);
    const previous = pointer.over;
    if (target !== previous) {
      if (previous && previous.isConnected) {
        for (const type of ['pointerout', 'pointerleave', 'mouseout', 'mouseleave']) {
          mouseEvent(previous, type, pointer, { relatedTarget: target });
        }
      }
      for (const type of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter']) {
        mouseEvent(target, type, pointer, { relatedTarget: previous });
      }
      pointer.over = target;
    }
    mouseEvent(target, 'pointermove', pointer);
    mouseEvent(target, 'mousemove', pointer);
  };
  const FOCUSABLE = 'a[href], area[href], button, input, select, textarea, iframe, summary, [tabindex], '
    + '[contenteditable]:not([contenteditable="false"])';
  // A press focuses what it lands on, or what holds that, as a real one does.
  const focusFrom = (target: El): void => {
    const focusable = target.closest<El>(FOCUSABLE);
    if (focusable && !focusable.disabled) focusable.focus({ preventScroll: true });
    else (target.ownerDocument.activeElement as HTMLElement | null)?.blur?.();
  };
  const pressPointer = (pointer: Pointer, { button }: { button: number }): void => {
    if (pointer.buttons.has(button)) return;
    const target = hit(pointer);
    const now = performance.now();
    const last = input.press;
    const count = last && last.button === button && last.target === target
      && now - last.time < DOUBLE_CLICK_MS ? last.count + 1 : 1;
    input.press = { button, target, time: now, count };
    pointer.buttons.add(button);
    pointer.down = { button, target, count };
    // A cancelled pointerdown holds back the mouse events that follow it.
    if (mouseEvent(target, 'pointerdown', pointer, { button })
      && mouseEvent(target, 'mousedown', pointer, { button, detail: count })) {
      focusFrom(target);
    }
  };
  // The nearest element that holds both `a` and `b`.
  const commonAncestor = (a: El, b: El): El | null => {
    for (let node: El | null = a; node; node = node.parentElement) if (node.contains(b)) return node;
    return null;
  };
  const releasePointer = (pointer: Pointer, { button }: { button: number }): void => {
    if (!pointer.buttons.has(button)) return;
    const target = hit(pointer);
    const { down } = pointer;
    const count = down ? down.count : 1;
    pointer.buttons.delete(button);
    pointer.down = null;
    mouseEvent(target, 'pointerup', pointer, { button });
    mouseEvent(target, 'mouseup', pointer, { button, detail: count });
    const pressed = down && down.button === button && down.target.isConnected ? down.target : null;
    const clicked = pressed && commonAncestor(pressed, target);
    if (!clicked) return;
    if (button === 0) {
      mouseEvent(clicked, 'click', pointer, { button, detail: count });
      if (count === 2) mouseEvent(clicked, 'dblclick', pointer, { button, detail: 2 });
    } else {
      mouseEvent(clicked, 'auxclick', pointer, { button, detail: count });
      if (button === 2) mouseEvent(clicked, 'contextmenu', pointer, { button, detail: count });
    }
  };
  // Key names of the WebDriver key codes (W3C "normalised key value").
  const KEYS: Record<string, string> = {
    '': 'Unidentified', '': 'Cancel', '': 'Help', '': 'Backspace',
    '': 'Tab', '': 'Clear', '': 'Enter', [ENTER]: 'Enter', '': 'Shift',
    '': 'Control', '': 'Alt', '': 'Pause', '': 'Escape', '': ' ',
    '': 'PageUp', '': 'PageDown', '': 'End', '': 'Home',
    '': 'ArrowLeft', '': 'ArrowUp', '': 'ArrowRight', '': 'ArrowDown',
    '': 'Insert', '': 'Delete', '': ';', '': '=', '': '*',
    '': '+', '': ',', '': '-', '': '.', '': '/', '': 'Meta',
    '': 'Shift', '': 'Control', '': 'Alt', '': 'Meta', '': 'PageUp',
    '': 'PageDown', '': 'End', '': 'Home', '': 'ArrowLeft',
    '': 'ArrowUp', '': 'ArrowRight', '': 'ArrowDown', '': 'Insert',
    '': 'Delete',
  };
  for (let digit = 0; digit <= 9; digit += 1) KEYS[String.fromCharCode(0xE01A + digit)] = String(digit);
  for (let number = 1; number <= 12; number += 1) KEYS[String.fromCharCode(0xE030 + number)] = `F${number}`;
  const CODES: Record<string, string> = {
    ' ': 'Space', Shift: 'ShiftLeft', Control: 'ControlLeft', Alt: 'AltLeft', Meta: 'MetaLeft',
  };
  const codeOf = (key: string): string => {
    if (/^[a-z]$/i.test(key)) return `Key${key.toUpperCase()}`;
    if (/^[0-9]$/.test(key)) return `Digit${key}`;
    return CODES[key] || (key.length > 1 ? key : '');
  };
  const focused = (): El => {
    const { doc } = context();
    return (doc.activeElement || doc.body) as El;
  };
  const keyEvent = (target: El, type: string, key: string, repeat = false): boolean => {
    const view = viewOf(target);
    return target.dispatchEvent(new view.KeyboardEvent(type, {
      key, code: codeOf(key), bubbles: true, cancelable: true, composed: true, view, repeat, ...modifiers(),
    }));
  };
  const NOT_TEXT = new Set(['button', 'checkbox', 'color', 'file', 'hidden', 'image', 'radio', 'range', 'reset', 'submit']);
  const editable = (element: El): boolean => element.isContentEditable || element.localName === 'textarea'
    || (element.localName === 'input' && !NOT_TEXT.has(element.type as string));
  // Edits the focused field as typing does, through `execCommand`, so the
  // page sees `beforeinput` and `input`; else sets its value at the end.
  const edit = (target: El, command: string, text?: string): void => {
    if (!editable(target) || target.readOnly || target.disabled) return;
    if (target.ownerDocument.execCommand(command, false, text)) return;
    if (target.isContentEditable) return;
    const value = target.value ?? '';
    if (command === 'insertText') setNative(target, value + text);
    else if (command === 'delete') setNative(target, value.slice(0, -1));
    else return;
    target.dispatchEvent(new (viewOf(target).Event)('input', { bubbles: true }));
  };
  const enter = (target: El): void => {
    if (target.localName === 'textarea') edit(target, 'insertText', '\n');
    else if (target.isContentEditable) edit(target, 'insertParagraph');
    else if (target.localName === 'input') target.form?.requestSubmit?.();
    else if (target.localName === 'button' || (target.localName === 'a' && target.href)) target.click();
  };
  const pressKey = ({ value }: { value: string }): void => {
    const key = KEYS[value] ?? value;
    // A WebDriver code names a key; any other value is the text it types.
    const text = KEYS[value] === undefined || key.length === 1 ? key : null;
    const target = focused();
    const repeat = input.keys.has(key);
    input.keys.add(key);
    if (!keyEvent(target, 'keydown', key, repeat)) return;
    const { ctrlKey, metaKey } = modifiers();
    if (text !== null && !ctrlKey && !metaKey) {
      if (keyEvent(target, 'keypress', key)) edit(target, 'insertText', text);
    } else if (key === 'Enter') {
      if (keyEvent(target, 'keypress', key)) enter(target);
    } else if (key === 'Backspace' || key === 'Delete') {
      edit(target, key === 'Backspace' ? 'delete' : 'forwardDelete');
    }
  };
  const releaseKey = ({ value }: { value: string }): void => {
    const key = KEYS[value] ?? value;
    if (!input.keys.has(key)) return;
    input.keys.delete(key);
    keyEvent(focused(), 'keyup', key);
  };
  const forgetInput = (): void => {
    input.keys.clear();
    input.pointers.clear();
    input.press = null;
  };

  const ops: Ops = {
    info: ({ what }) => {
      switch (what) {
        case 'url': return location.href;
        case 'title': return document.title;
        case 'source': return context().doc.documentElement.outerHTML;
        case 'ready': return leaving ? 'leaving' : document.readyState;
        default: return fail('invalid argument', String(what));
      }
    },
    go: ({ url }) => {
      // A navigation leaves any frame for the top page.
      frames = [];
      const target = new URL(url, location.href);
      const here = new URL(location.href);
      target.hash = '';
      here.hash = '';
      // A change of the hash alone does not unload the page.
      if (target.href === here.href) {
        location.href = url;
        return null;
      }
      return leave(() => { location.href = url; });
    },
    history: ({ action }) => leave(() => {
      if (action === 'refresh') location.reload();
      else history[action]();
    }),
    find: ({ using, value, root, all }) => find(root ? get(root) : context().doc, using, value, Boolean(all)),
    active: () => {
      const { doc } = context();
      return { [KEY]: ref((doc.activeElement || doc.body) as El) };
    },
    prop: ({ id, what, name }) => {
      const element = get(id);
      switch (what) {
        case 'text': return element.innerText ?? element.textContent;
        case 'name': return element.tagName.toLowerCase();
        case 'attribute': return element.getAttribute(name);
        case 'property': return wrap((element as unknown as Obj)[name]);
        case 'css': return viewOf(element).getComputedStyle(element).getPropertyValue(name);
        case 'enabled': return !element.disabled;
        case 'selected': return Boolean(element.checked || element.selected);
        case 'displayed': return visible(element);
        case 'rect': {
          const rect = element.getBoundingClientRect();
          const view = viewOf(element);
          return { x: rect.x + view.scrollX, y: rect.y + view.scrollY, width: rect.width, height: rect.height };
        }
        case 'viewrect': return viewportRect(element);
        default: return fail('unknown command', what);
      }
    },
    click: ({ id }) => {
      const element = get(id);
      scrollToCentre(element);
      if (!visible(element)) fail('element not interactable', 'the element is not visible');
      element.focus?.();
      element.click();
      return null;
    },
    clear: ({ id }) => {
      setValue(get(id), '');
      return null;
    },
    keys: ({ id, text }) => {
      const element = get(id);
      element.focus?.();
      const parts = text.split(ENTER);
      setValue(element, (element.value ?? element.textContent ?? '') + parts.join(''));
      if (parts.length > 1) {
        const { KeyboardEvent } = viewOf(element);
        for (const type of ['keydown', 'keypress', 'keyup']) {
          element.dispatchEvent(new KeyboardEvent(type, { key: 'Enter', code: 'Enter', bubbles: true }));
        }
        element.form?.requestSubmit?.();
      }
      return null;
    },
    // A script runs in the current frame: its `window` and `document`.
    script: async ({ script, args, isAsync }) => {
      const { win } = context();
      const run = new (win as unknown as { Function: FunctionConstructor }).Function(script);
      const values = unwrap(args || []) as unknown[];
      if (!isAsync) return wrap(await run.apply(win, values));
      return wrap(await new Promise((resolve) => run.apply(win, [...values, resolve])));
    },
    dialog: ({ action, text }) => {
      const [open] = dialogs.open;
      if (action === 'text') {
        if (!open) fail('no such alert', 'no dialog is open: none has opened, or each was accepted or dismissed');
        return open.text;
      }
      if (action === 'type') {
        if (open && open.type !== 'prompt') fail('element not interactable', `a ${open.type} takes no text`);
        if (open) {
          fail('unsupported operation', 'the prompt was answered when it opened: send its text and accept before it opens');
        }
        dialogs.text = String(text);
        return null;
      }
      // With no dialog open, accept or dismiss is the answer for the next
      // confirm or prompt, in place of any set before. With one open, they
      // clear the oldest, but only as the page was answered: the page has its
      // answer and cannot be given another.
      const accept = action === 'accept';
      if (open && open.type !== 'alert' && open.accepted !== accept) {
        fail('unsupported operation', `the ${open.type} was ${open.accepted ? 'accepted' : 'dismissed'} when it opened, `
          + `and the page cannot be answered again: ${action} before the ${open.type} opens`);
      }
      if (open) dialogs.open.shift();
      else dialogs.answer = { accept, text: dialogs.text };
      dialogs.text = null;
      return null;
    },
    frame: ({ top, parent, index, element }) => {
      if (top) {
        frames = [];
        return null;
      }
      if (parent) {
        frames.pop();
        return null;
      }
      const { win, doc } = context();
      let frame: El | undefined;
      if (element !== undefined) {
        frame = get(element);
        if (!['iframe', 'frame'].includes(frame.localName)) fail('no such frame', 'the element is not a frame');
        if (frame.ownerDocument !== doc) fail('no such frame', 'the frame element is not in the current frame');
      } else {
        const child = index !== undefined && index < win.length ? win[index] ?? null : null;
        if (!child) fail('no such frame', `the current page has ${win.length} frames, none at index ${index}`);
        frame = [...doc.querySelectorAll<El>('iframe, frame')]
          .find((candidate) => (candidate as unknown as HTMLIFrameElement).contentWindow === child);
        if (!frame) fail('no such frame', `frame ${index} is not an iframe or frame element`);
      }
      const entered = frame as unknown as HTMLIFrameElement;
      if (!frameDocument(entered)) {
        fail('no such frame', 'the frame is cross-origin: only a frame of the same origin as the app\'s page can be entered, since the page script cannot reach into another origin');
      }
      frames.push(entered);
      hookDialogs(entered.contentWindow as Window);
      return null;
    },
    input: ({ actions }) => {
      for (const action of actions) {
        switch (action.type) {
          case 'pointerMove': movePointer(pointerOf(action.source), action); break;
          case 'pointerDown': pressPointer(pointerOf(action.source), action); break;
          case 'pointerUp': releasePointer(pointerOf(action.source), action); break;
          case 'keyDown': pressKey(action); break;
          case 'keyUp': releaseKey(action); break;
          default: fail('unsupported operation', `\`${(action as { type: string }).type}\` is not simulated`);
        }
      }
      return null;
    },
    // Lets go of the keys and buttons held, last pressed first, then forgets
    // the input state.
    release: () => {
      try {
        // The frame entered may be gone: no event can reach it, and Release
        // Actions does not fail for that, so only the state is forgotten.
        try {
          context();
        } catch {
          return null;
        }
        for (const key of [...input.keys].reverse()) releaseKey({ value: key });
        for (const pointer of input.pointers.values()) {
          for (const button of [...pointer.buttons].reverse()) releasePointer(pointer, { button });
        }
      } finally {
        forgetInput();
      }
      return null;
    },
    // A new session: the top page, no dialog answers, no input held.
    reset: () => {
      frames = [];
      dialogs.open = [];
      dialogs.answer = null;
      dialogs.text = null;
      forgetInput();
      return null;
    },
  };

  const reply = (payload: unknown): Promise<unknown> => page.__TAURI_INTERNALS__.invoke('plugin:sbrs-automation|reply', payload);
  Object.defineProperty(window, '__sbrs', {
    value: Object.freeze({
      run: async (id: unknown, op: string, args?: unknown): Promise<void> => {
        try {
          if (!Object.hasOwn(ops, op)) fail('unknown command', op);
          // Frames load on their own; hook the dialogs of any new one.
          hookFrames(window);
          const value = await (ops[op as keyof Ops] as (args: unknown) => unknown)(args || {});
          await reply({ id, ok: true, value: value === undefined ? null : value });
        } catch (error) {
          const caught = error as Partial<BridgeError> | null | undefined;
          const code = caught && caught.code ? caught.code : 'javascript error';
          await reply({ id, ok: false, value: { error: code, message: String((caught && caught.message) || error) } });
        }
      },
    }),
  });
})();
