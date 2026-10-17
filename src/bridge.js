// Generated from src/bridge.ts (sha256: 1a1b8df9d803d27beb05b1b968857210a03012278489777c5d6deeb9b9aab32c); edit bridge.ts and run "bun run bridge".
(() => {
  // src/bridge.ts
  (() => {
    const page = window;
    if (page.__sbrs || window !== window.top)
      return;
    const KEY = "element-6066-11e4-a52e-4f735466cecf";
    const ENTER = "";
    const elements = new Map;
    let next = 0;
    let leaving = false;
    let frames = [];
    addEventListener("hashchange", () => {
      leaving = false;
    });
    addEventListener("popstate", () => {
      leaving = false;
    });
    function fail(code, message) {
      const error = new Error(message);
      error.code = code;
      throw error;
    }
    const isElement = (value) => value !== null && typeof value === "object" && value.nodeType === 1 && typeof value.tagName === "string";
    const isList = (value) => Array.isArray(value) || ["[object NodeList]", "[object HTMLCollection]"].includes(Object.prototype.toString.call(value));
    const isWindow = (value) => value !== null && typeof value === "object" && value.window === value;
    const viewOf = (element) => element.ownerDocument?.defaultView || window;
    const ref = (element) => {
      for (const [id, known] of elements)
        if (known === element)
          return id;
      const id = `e${++next}-${Math.random().toString(36).slice(2, 10)}`;
      elements.set(id, element);
      return id;
    };
    const get = (id) => {
      const element = elements.get(id);
      if (!element)
        fail("no such element", `no element ${id} in this page`);
      if (!element.isConnected)
        fail("stale element reference", `element ${id} is no longer in the page`);
      return element;
    };
    const wrap = (value) => {
      if (isElement(value))
        return { [KEY]: ref(value) };
      if (isList(value))
        return Array.from(value, wrap);
      if (value && typeof value === "object" && !isWindow(value)) {
        const out = {};
        for (const key of Object.keys(value))
          out[key] = wrap(value[key]);
        return out;
      }
      return value === undefined ? null : value;
    };
    const unwrap = (value) => {
      if (Array.isArray(value))
        return value.map(unwrap);
      if (value && typeof value === "object") {
        const object = value;
        if (typeof object[KEY] === "string")
          return get(object[KEY]);
        const out = {};
        for (const key of Object.keys(object))
          out[key] = unwrap(object[key]);
        return out;
      }
      return value;
    };
    const visible = (element) => {
      const style = viewOf(element).getComputedStyle(element);
      if (style.display === "none" || style.visibility === "hidden" || style.opacity === "0")
        return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const setNative = (element, value) => {
      const view = viewOf(element);
      const proto = element instanceof view.HTMLTextAreaElement ? view.HTMLTextAreaElement.prototype : element instanceof view.HTMLInputElement ? view.HTMLInputElement.prototype : null;
      const setter = proto && Object.getOwnPropertyDescriptor(proto, "value")?.set;
      if (setter)
        setter.call(element, value);
      else if (element.isContentEditable)
        element.textContent = value;
      else
        element.value = value;
    };
    const setValue = (element, value) => {
      setNative(element, value);
      const view = viewOf(element);
      element.dispatchEvent(new view.Event("input", { bubbles: true }));
      element.dispatchEvent(new view.Event("change", { bubbles: true }));
    };
    function find(root, using, value, all) {
      let found = [];
      try {
        switch (using) {
          case "css selector":
            found = all ? [...root.querySelectorAll(value)] : [root.querySelector(value)];
            break;
          case "tag name":
            found = [...root.getElementsByTagName(value)];
            break;
          case "link text":
          case "partial link text":
            found = [...root.querySelectorAll("a")].filter((a) => using === "link text" ? a.innerText.trim() === value : a.innerText.includes(value));
            break;
          case "xpath": {
            const result = (root.ownerDocument || root).evaluate(value, root, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
            for (let i = 0;i < result.snapshotLength; i += 1)
              found.push(result.snapshotItem(i));
            break;
          }
          default:
            fail("invalid argument", `unknown locator strategy \`${using}\``);
        }
      } catch (error) {
        const caught = error;
        if (caught?.code)
          throw error;
        fail("invalid selector", String(caught?.message || error));
      }
      const elementsFound = found.filter(isElement);
      if (all)
        return elementsFound.map((node) => ({ [KEY]: ref(node) }));
      const [first] = elementsFound;
      if (!first)
        fail("no such element", `no element matches ${using} \`${value}\``);
      return { [KEY]: ref(first) };
    }
    const leave = (go) => {
      leaving = true;
      frames = [];
      setTimeout(go, 0);
      return null;
    };
    const frameDocument = (frame) => {
      try {
        return frame.contentDocument;
      } catch {
        return null;
      }
    };
    const context = () => {
      const frame = frames.at(-1);
      if (!frame)
        return { win: window, doc: document };
      const doc = frame.isConnected ? frameDocument(frame) : null;
      if (!doc || !doc.defaultView) {
        fail("no such frame", "the frame entered is gone or is no longer same-origin; switch to its parent or the top page");
      }
      return { win: doc.defaultView, doc };
    };
    const scrollToCentre = (element) => {
      element.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    };
    const viewportRect = (element) => {
      scrollToCentre(element);
      let { left, top, right, bottom } = element.getBoundingClientRect();
      const clip = (view) => {
        left = Math.max(left, 0);
        top = Math.max(top, 0);
        right = Math.min(right, view.innerWidth);
        bottom = Math.min(bottom, view.innerHeight);
      };
      for (let view = viewOf(element);view !== window && view.frameElement; view = view.parent) {
        clip(view);
        const frame = view.frameElement;
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
      if (right <= left || bottom <= top)
        fail("unable to capture screen", "the element has no area in view");
      return { x: left, y: top, width: right - left, height: bottom - top };
    };
    const dialogs = {
      open: [],
      answer: null,
      text: null
    };
    const MAX_DIALOGS = 100;
    const ask = (type, message, fallback) => {
      let answer = { accept: true, text: null };
      if (type !== "alert") {
        answer = dialogs.answer || { accept: false, text: null };
        dialogs.answer = null;
      }
      dialogs.open.push({ type, text: message === undefined ? "" : String(message), accepted: answer.accept });
      if (dialogs.open.length > MAX_DIALOGS)
        dialogs.open.shift();
      if (type === "alert")
        return;
      if (type === "confirm")
        return answer.accept;
      if (!answer.accept)
        return null;
      if (answer.text !== null)
        return answer.text;
      return fallback === undefined ? "" : String(fallback);
    };
    const HOOKED = Symbol("sbrs-automation dialogs");
    const hookDialogs = (win) => {
      try {
        if (win[HOOKED])
          return;
        Object.defineProperty(win, HOOKED, { value: true });
        win.alert = (message) => {
          ask("alert", message);
        };
        win.confirm = (message) => ask("confirm", message);
        win.prompt = (message, value) => ask("prompt", message, value);
      } catch {}
    };
    const hookFrames = (win) => {
      for (let index = 0;index < win.length; index += 1) {
        const child = win[index];
        let reachable = false;
        try {
          reachable = Boolean(child?.document);
        } catch {}
        if (child && reachable) {
          hookDialogs(child);
          hookFrames(child);
        }
      }
    };
    hookDialogs(window);
    const input = {
      keys: new Set,
      pointers: new Map,
      press: null
    };
    const DOUBLE_CLICK_MS = 500;
    const pointerOf = (source) => {
      let pointer = input.pointers.get(source);
      if (!pointer) {
        pointer = { x: 0, y: 0, over: null, buttons: new Set, down: null };
        input.pointers.set(source, pointer);
      }
      return pointer;
    };
    const modifiers = () => ({
      shiftKey: input.keys.has("Shift"),
      ctrlKey: input.keys.has("Control"),
      altKey: input.keys.has("Alt"),
      metaKey: input.keys.has("Meta")
    });
    const buttonMask = (pressed) => [...pressed].reduce((mask, button) => mask | ([1, 4, 2, 8, 16][button] ?? 0), 0);
    const NOT_BUBBLING = new Set(["mouseenter", "mouseleave", "pointerenter", "pointerleave"]);
    const mouseEvent = (target, type, pointer, extra = {}) => {
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
        ...extra
      };
      if (type.startsWith("pointer") && view.PointerEvent) {
        return target.dispatchEvent(new view.PointerEvent(type, {
          pointerId: 1,
          pointerType: "mouse",
          isPrimary: true,
          ...init
        }));
      }
      return target.dispatchEvent(new view.MouseEvent(type, init));
    };
    const hit = (pointer) => {
      const { doc } = context();
      return doc.elementFromPoint(pointer.x, pointer.y) || doc.documentElement;
    };
    const movePointer = (pointer, { origin, x, y }) => {
      const { win, doc } = context();
      let left = x;
      let top = y;
      if (origin === "pointer") {
        left += pointer.x;
        top += pointer.y;
      } else if (origin !== "viewport") {
        const element = get(origin.element);
        if (element.ownerDocument !== doc)
          fail("no such element", "the element is not in the current frame");
        let rect = element.getBoundingClientRect();
        const centre = () => [rect.left + rect.width / 2, rect.top + rect.height / 2];
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
        fail("move target out of bounds", `(${left}, ${top}) is outside the viewport, ${win.innerWidth} x ${win.innerHeight}`);
      }
      pointer.x = left;
      pointer.y = top;
      const target = hit(pointer);
      const previous = pointer.over;
      if (target !== previous) {
        if (previous && previous.isConnected) {
          for (const type of ["pointerout", "pointerleave", "mouseout", "mouseleave"]) {
            mouseEvent(previous, type, pointer, { relatedTarget: target });
          }
        }
        for (const type of ["pointerover", "pointerenter", "mouseover", "mouseenter"]) {
          mouseEvent(target, type, pointer, { relatedTarget: previous });
        }
        pointer.over = target;
      }
      mouseEvent(target, "pointermove", pointer);
      mouseEvent(target, "mousemove", pointer);
    };
    const FOCUSABLE = "a[href], area[href], button, input, select, textarea, iframe, summary, [tabindex], " + '[contenteditable]:not([contenteditable="false"])';
    const focusFrom = (target) => {
      const focusable = target.closest(FOCUSABLE);
      if (focusable && !focusable.disabled)
        focusable.focus({ preventScroll: true });
      else
        target.ownerDocument.activeElement?.blur?.();
    };
    const pressPointer = (pointer, { button }) => {
      if (pointer.buttons.has(button))
        return;
      const target = hit(pointer);
      const now = performance.now();
      const last = input.press;
      const count = last && last.button === button && last.target === target && now - last.time < DOUBLE_CLICK_MS ? last.count + 1 : 1;
      input.press = { button, target, time: now, count };
      pointer.buttons.add(button);
      pointer.down = { button, target, count };
      if (mouseEvent(target, "pointerdown", pointer, { button }) && mouseEvent(target, "mousedown", pointer, { button, detail: count })) {
        focusFrom(target);
      }
    };
    const commonAncestor = (a, b) => {
      for (let node = a;node; node = node.parentElement)
        if (node.contains(b))
          return node;
      return null;
    };
    const releasePointer = (pointer, { button }) => {
      if (!pointer.buttons.has(button))
        return;
      const target = hit(pointer);
      const { down } = pointer;
      const count = down ? down.count : 1;
      pointer.buttons.delete(button);
      pointer.down = null;
      mouseEvent(target, "pointerup", pointer, { button });
      mouseEvent(target, "mouseup", pointer, { button, detail: count });
      const pressed = down && down.button === button && down.target.isConnected ? down.target : null;
      const clicked = pressed && commonAncestor(pressed, target);
      if (!clicked)
        return;
      if (button === 0) {
        mouseEvent(clicked, "click", pointer, { button, detail: count });
        if (count === 2)
          mouseEvent(clicked, "dblclick", pointer, { button, detail: 2 });
      } else {
        mouseEvent(clicked, "auxclick", pointer, { button, detail: count });
        if (button === 2)
          mouseEvent(clicked, "contextmenu", pointer, { button, detail: count });
      }
    };
    const KEYS = {
      "": "Unidentified",
      "": "Cancel",
      "": "Help",
      "": "Backspace",
      "": "Tab",
      "": "Clear",
      "": "Enter",
      [ENTER]: "Enter",
      "": "Shift",
      "": "Control",
      "": "Alt",
      "": "Pause",
      "": "Escape",
      "": " ",
      "": "PageUp",
      "": "PageDown",
      "": "End",
      "": "Home",
      "": "ArrowLeft",
      "": "ArrowUp",
      "": "ArrowRight",
      "": "ArrowDown",
      "": "Insert",
      "": "Delete",
      "": ";",
      "": "=",
      "": "*",
      "": "+",
      "": ",",
      "": "-",
      "": ".",
      "": "/",
      "": "Meta",
      "": "Shift",
      "": "Control",
      "": "Alt",
      "": "Meta",
      "": "PageUp",
      "": "PageDown",
      "": "End",
      "": "Home",
      "": "ArrowLeft",
      "": "ArrowUp",
      "": "ArrowRight",
      "": "ArrowDown",
      "": "Insert",
      "": "Delete"
    };
    for (let digit = 0;digit <= 9; digit += 1)
      KEYS[String.fromCharCode(57370 + digit)] = String(digit);
    for (let number = 1;number <= 12; number += 1)
      KEYS[String.fromCharCode(57392 + number)] = `F${number}`;
    const CODES = {
      " ": "Space",
      Shift: "ShiftLeft",
      Control: "ControlLeft",
      Alt: "AltLeft",
      Meta: "MetaLeft"
    };
    const codeOf = (key) => {
      if (/^[a-z]$/i.test(key))
        return `Key${key.toUpperCase()}`;
      if (/^[0-9]$/.test(key))
        return `Digit${key}`;
      return CODES[key] || (key.length > 1 ? key : "");
    };
    const focused = () => {
      const { doc } = context();
      return doc.activeElement || doc.body;
    };
    const keyEvent = (target, type, key, repeat = false) => {
      const view = viewOf(target);
      return target.dispatchEvent(new view.KeyboardEvent(type, {
        key,
        code: codeOf(key),
        bubbles: true,
        cancelable: true,
        composed: true,
        view,
        repeat,
        ...modifiers()
      }));
    };
    const NOT_TEXT = new Set(["button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit"]);
    const editable = (element) => element.isContentEditable || element.localName === "textarea" || element.localName === "input" && !NOT_TEXT.has(element.type);
    const edit = (target, command, text) => {
      if (!editable(target) || target.readOnly || target.disabled)
        return;
      if (target.ownerDocument.execCommand(command, false, text))
        return;
      if (target.isContentEditable)
        return;
      const value = target.value ?? "";
      if (command === "insertText")
        setNative(target, value + text);
      else if (command === "delete")
        setNative(target, value.slice(0, -1));
      else
        return;
      target.dispatchEvent(new (viewOf(target)).Event("input", { bubbles: true }));
    };
    const enter = (target) => {
      if (target.localName === "textarea")
        edit(target, "insertText", `
`);
      else if (target.isContentEditable)
        edit(target, "insertParagraph");
      else if (target.localName === "input")
        target.form?.requestSubmit?.();
      else if (target.localName === "button" || target.localName === "a" && target.href)
        target.click();
    };
    const pressKey = ({ value }) => {
      const key = KEYS[value] ?? value;
      const text = KEYS[value] === undefined || key.length === 1 ? key : null;
      const target = focused();
      const repeat = input.keys.has(key);
      input.keys.add(key);
      if (!keyEvent(target, "keydown", key, repeat))
        return;
      const { ctrlKey, metaKey } = modifiers();
      if (text !== null && !ctrlKey && !metaKey) {
        if (keyEvent(target, "keypress", key))
          edit(target, "insertText", text);
      } else if (key === "Enter") {
        if (keyEvent(target, "keypress", key))
          enter(target);
      } else if (key === "Backspace" || key === "Delete") {
        edit(target, key === "Backspace" ? "delete" : "forwardDelete");
      }
    };
    const releaseKey = ({ value }) => {
      const key = KEYS[value] ?? value;
      if (!input.keys.has(key))
        return;
      input.keys.delete(key);
      keyEvent(focused(), "keyup", key);
    };
    const forgetInput = () => {
      input.keys.clear();
      input.pointers.clear();
      input.press = null;
    };
    const ops = {
      info: ({ what }) => {
        switch (what) {
          case "url":
            return location.href;
          case "title":
            return document.title;
          case "source":
            return context().doc.documentElement.outerHTML;
          case "ready":
            return leaving ? "leaving" : document.readyState;
          default:
            return fail("invalid argument", String(what));
        }
      },
      go: ({ url }) => {
        frames = [];
        const target = new URL(url, location.href);
        const here = new URL(location.href);
        target.hash = "";
        here.hash = "";
        if (target.href === here.href) {
          location.href = url;
          return null;
        }
        return leave(() => {
          location.href = url;
        });
      },
      history: ({ action }) => leave(() => {
        if (action === "refresh")
          location.reload();
        else
          history[action]();
      }),
      find: ({ using, value, root, all }) => find(root ? get(root) : context().doc, using, value, Boolean(all)),
      active: () => {
        const { doc } = context();
        return { [KEY]: ref(doc.activeElement || doc.body) };
      },
      prop: ({ id, what, name }) => {
        const element = get(id);
        switch (what) {
          case "text":
            return element.innerText ?? element.textContent;
          case "name":
            return element.tagName.toLowerCase();
          case "attribute":
            return element.getAttribute(name);
          case "property":
            return wrap(element[name]);
          case "css":
            return viewOf(element).getComputedStyle(element).getPropertyValue(name);
          case "enabled":
            return !element.disabled;
          case "selected":
            return Boolean(element.checked || element.selected);
          case "displayed":
            return visible(element);
          case "rect": {
            const rect = element.getBoundingClientRect();
            const view = viewOf(element);
            return { x: rect.x + view.scrollX, y: rect.y + view.scrollY, width: rect.width, height: rect.height };
          }
          case "viewrect":
            return viewportRect(element);
          default:
            return fail("unknown command", what);
        }
      },
      click: ({ id }) => {
        const element = get(id);
        scrollToCentre(element);
        if (!visible(element))
          fail("element not interactable", "the element is not visible");
        element.focus?.();
        element.click();
        return null;
      },
      clear: ({ id }) => {
        setValue(get(id), "");
        return null;
      },
      keys: ({ id, text }) => {
        const element = get(id);
        element.focus?.();
        const parts = text.split(ENTER);
        setValue(element, (element.value ?? element.textContent ?? "") + parts.join(""));
        if (parts.length > 1) {
          const { KeyboardEvent } = viewOf(element);
          for (const type of ["keydown", "keypress", "keyup"]) {
            element.dispatchEvent(new KeyboardEvent(type, { key: "Enter", code: "Enter", bubbles: true }));
          }
          element.form?.requestSubmit?.();
        }
        return null;
      },
      script: async ({ script, args, isAsync }) => {
        const { win } = context();
        const run = new win.Function(script);
        const values = unwrap(args || []);
        if (!isAsync)
          return wrap(await run.apply(win, values));
        return wrap(await new Promise((resolve) => run.apply(win, [...values, resolve])));
      },
      dialog: ({ action, text }) => {
        const [open] = dialogs.open;
        if (action === "text") {
          if (!open)
            fail("no such alert", "no dialog is open: none has opened, or each was accepted or dismissed");
          return open.text;
        }
        if (action === "type") {
          if (open && open.type !== "prompt")
            fail("element not interactable", `a ${open.type} takes no text`);
          if (open) {
            fail("unsupported operation", "the prompt was answered when it opened: send its text and accept before it opens");
          }
          dialogs.text = String(text);
          return null;
        }
        const accept = action === "accept";
        if (open && open.type !== "alert" && open.accepted !== accept) {
          fail("unsupported operation", `the ${open.type} was ${open.accepted ? "accepted" : "dismissed"} when it opened, ` + `and the page cannot be answered again: ${action} before the ${open.type} opens`);
        }
        if (open)
          dialogs.open.shift();
        else
          dialogs.answer = { accept, text: dialogs.text };
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
        let frame;
        if (element !== undefined) {
          frame = get(element);
          if (!["iframe", "frame"].includes(frame.localName))
            fail("no such frame", "the element is not a frame");
          if (frame.ownerDocument !== doc)
            fail("no such frame", "the frame element is not in the current frame");
        } else {
          const child = index !== undefined && index < win.length ? win[index] ?? null : null;
          if (!child)
            fail("no such frame", `the current page has ${win.length} frames, none at index ${index}`);
          frame = [...doc.querySelectorAll("iframe, frame")].find((candidate) => candidate.contentWindow === child);
          if (!frame)
            fail("no such frame", `frame ${index} is not an iframe or frame element`);
        }
        const entered = frame;
        if (!frameDocument(entered)) {
          fail("no such frame", "the frame is cross-origin: only a frame of the same origin as the app's page can be entered, since the page script cannot reach into another origin");
        }
        frames.push(entered);
        hookDialogs(entered.contentWindow);
        return null;
      },
      input: ({ actions }) => {
        for (const action of actions) {
          switch (action.type) {
            case "pointerMove":
              movePointer(pointerOf(action.source), action);
              break;
            case "pointerDown":
              pressPointer(pointerOf(action.source), action);
              break;
            case "pointerUp":
              releasePointer(pointerOf(action.source), action);
              break;
            case "keyDown":
              pressKey(action);
              break;
            case "keyUp":
              releaseKey(action);
              break;
            default:
              fail("unsupported operation", `\`${action.type}\` is not simulated`);
          }
        }
        return null;
      },
      release: () => {
        try {
          try {
            context();
          } catch {
            return null;
          }
          for (const key of [...input.keys].reverse())
            releaseKey({ value: key });
          for (const pointer of input.pointers.values()) {
            for (const button of [...pointer.buttons].reverse())
              releasePointer(pointer, { button });
          }
        } finally {
          forgetInput();
        }
        return null;
      },
      reset: () => {
        frames = [];
        dialogs.open = [];
        dialogs.answer = null;
        dialogs.text = null;
        forgetInput();
        return null;
      }
    };
    const reply = (payload) => page.__TAURI_INTERNALS__.invoke("plugin:sbrs-automation|reply", payload);
    Object.defineProperty(window, "__sbrs", {
      value: Object.freeze({
        run: async (id, op, args) => {
          try {
            if (!Object.hasOwn(ops, op))
              fail("unknown command", op);
            hookFrames(window);
            const value = await ops[op](args || {});
            await reply({ id, ok: true, value: value === undefined ? null : value });
          } catch (error) {
            const caught = error;
            const code = caught && caught.code ? caught.code : "javascript error";
            await reply({ id, ok: false, value: { error: code, message: String(caught && caught.message || error) } });
          }
        }
      })
    });
  })();
})();
