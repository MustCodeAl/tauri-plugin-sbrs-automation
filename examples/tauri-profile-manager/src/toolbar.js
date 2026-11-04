// The toolbar of a profile's app window. It runs in its own webview, above the
// page: the page never sees it, and only it can ask the app to move the page
// (`control_profile_window`, which acts over the page's DevTools channel).
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const address = document.getElementById("address");
const reload = document.getElementById("reload");
let loading = false;

function act(action) {
  invoke("control_profile_window", { action }).catch(showError);
}

function showError(error) {
  address.setCustomValidity(String(error));
  address.reportValidity();
}

function setLoading(on) {
  loading = on;
  reload.textContent = on ? "✕" : "↻";
  reload.title = on ? "Stop" : "Reload (⌘R)";
  reload.setAttribute("aria-label", on ? "Stop" : "Reload");
}

document.getElementById("back").addEventListener("click", () => act("back"));
document.getElementById("forward").addEventListener("click", () => act("forward"));
reload.addEventListener("click", () => act(loading ? "stop" : "reload"));

document.getElementById("go").addEventListener("submit", (event) => {
  event.preventDefault();
  address.setCustomValidity("");
  invoke("control_profile_window", { address: address.value })
    .then(() => address.blur())
    .catch(showError);
});

address.addEventListener("input", () => address.setCustomValidity(""));
address.addEventListener("focus", () => address.select());

document.addEventListener("keydown", (event) => {
  if (!event.metaKey && !event.ctrlKey) return;
  const keys = { "[": "back", "]": "forward", r: "reload" };
  if (keys[event.key]) {
    event.preventDefault();
    act(keys[event.key]);
  } else if (event.key === "l") {
    event.preventDefault();
    address.focus();
  }
});

// Each load of the page, from the app: its address and whether it is loading.
listen("profile-page", ({ payload }) => {
  if (document.activeElement !== address) {
    address.value = payload.url === "about:blank" ? "" : payload.url;
  }
  setLoading(payload.loading);
});
