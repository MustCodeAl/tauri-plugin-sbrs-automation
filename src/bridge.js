// Injected into every page of the app by tauri-plugin-sbrs-automation. Runs one
// WebDriver operation at a time for the plugin and sends the result back over
// Tauri's IPC (the plugin's `reply` command).
(() => {
  if (window.__sbrs) return;
  const KEY = 'element-6066-11e4-a52e-4f735466cecf';
  const ENTER = '';
  const elements = new Map();
  let next = 0;
  let leaving = false;
  addEventListener('hashchange', () => { leaving = false; });
  addEventListener('popstate', () => { leaving = false; });

  const fail = (code, message) => {
    const error = new Error(message);
    error.code = code;
    throw error;
  };
  const ref = (element) => {
    for (const [id, known] of elements) if (known === element) return id;
    const id = `e${++next}-${Math.random().toString(36).slice(2, 10)}`;
    elements.set(id, element);
    return id;
  };
  const get = (id) => {
    const element = elements.get(id);
    if (!element) fail('no such element', `no element ${id} in this page`);
    if (!element.isConnected) fail('stale element reference', `element ${id} is no longer in the page`);
    return element;
  };
  const wrap = (value) => {
    if (value instanceof Element) return { [KEY]: ref(value) };
    if (value instanceof NodeList || value instanceof HTMLCollection || Array.isArray(value)) {
      return Array.from(value, wrap);
    }
    if (value && typeof value === 'object' && !(value instanceof Window)) {
      const out = {};
      for (const key of Object.keys(value)) out[key] = wrap(value[key]);
      return out;
    }
    return value === undefined ? null : value;
  };
  const unwrap = (value) => {
    if (Array.isArray(value)) return value.map(unwrap);
    if (value && typeof value === 'object') {
      if (typeof value[KEY] === 'string') return get(value[KEY]);
      const out = {};
      for (const key of Object.keys(value)) out[key] = unwrap(value[key]);
      return out;
    }
    return value;
  };
  const visible = (element) => {
    const style = getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  // React and similar frameworks watch the native value setter, so use it.
  const setValue = (element, value) => {
    const proto = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype
      : element instanceof HTMLInputElement ? HTMLInputElement.prototype : null;
    const setter = proto && Object.getOwnPropertyDescriptor(proto, 'value')?.set;
    if (setter) setter.call(element, value);
    else if (element.isContentEditable) element.textContent = value;
    else element.value = value;
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const find = (root, using, value, all) => {
    let found = [];
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
          const result = document.evaluate(value, root, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
          for (let i = 0; i < result.snapshotLength; i += 1) found.push(result.snapshotItem(i));
          break;
        }
        default:
          fail('invalid argument', `unknown locator strategy \`${using}\``);
      }
    } catch (error) {
      if (error.code) throw error;
      fail('invalid selector', String(error.message || error));
    }
    found = found.filter((node) => node instanceof Element);
    if (all) return found.map((node) => ({ [KEY]: ref(node) }));
    if (!found.length) fail('no such element', `no element matches ${using} \`${value}\``);
    return { [KEY]: ref(found[0]) };
  };
  const leave = (go) => {
    leaving = true;
    setTimeout(go, 0);
    return null;
  };

  const ops = {
    info: ({ what }) => {
      switch (what) {
        case 'url': return location.href;
        case 'title': return document.title;
        case 'source': return document.documentElement.outerHTML;
        case 'ready': return leaving ? 'leaving' : document.readyState;
        default: return fail('invalid argument', what);
      }
    },
    go: ({ url }) => {
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
    find: ({ using, value, root, all }) => find(root ? get(root) : document, using, value, all),
    active: () => ({ [KEY]: ref(document.activeElement || document.body) }),
    prop: ({ id, what, name }) => {
      const element = get(id);
      switch (what) {
        case 'text': return element.innerText ?? element.textContent;
        case 'name': return element.tagName.toLowerCase();
        case 'attribute': return element.getAttribute(name);
        case 'property': return wrap(element[name]);
        case 'css': return getComputedStyle(element).getPropertyValue(name);
        case 'enabled': return !element.disabled;
        case 'selected': return Boolean(element.checked || element.selected);
        case 'displayed': return visible(element);
        case 'rect': {
          const rect = element.getBoundingClientRect();
          return { x: rect.x + scrollX, y: rect.y + scrollY, width: rect.width, height: rect.height };
        }
        default: return fail('unknown command', what);
      }
    },
    click: ({ id }) => {
      const element = get(id);
      element.scrollIntoView({ block: 'center', inline: 'center' });
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
        for (const type of ['keydown', 'keypress', 'keyup']) {
          element.dispatchEvent(new KeyboardEvent(type, { key: 'Enter', code: 'Enter', bubbles: true }));
        }
        element.form?.requestSubmit?.();
      }
      return null;
    },
    rect: () => ({ x: screenX, y: screenY, width: outerWidth, height: outerHeight }),
    script: async ({ script, args, isAsync }) => {
      const run = new Function(script);
      const values = unwrap(args || []);
      if (!isAsync) return wrap(await run.apply(window, values));
      return wrap(await new Promise((resolve) => run.apply(window, [...values, resolve])));
    },
  };

  const reply = (payload) => window.__TAURI_INTERNALS__.invoke('plugin:sbrs-automation|reply', payload);
  Object.defineProperty(window, '__sbrs', {
    value: Object.freeze({
      run: async (id, op, args) => {
        try {
          if (!Object.hasOwn(ops, op)) fail('unknown command', op);
          const value = await ops[op](args || {});
          await reply({ id, ok: true, value: value === undefined ? null : value });
        } catch (error) {
          const code = error && error.code ? error.code : 'javascript error';
          await reply({ id, ok: false, value: { error: code, message: String((error && error.message) || error) } });
        }
      },
    }),
  });
})();
