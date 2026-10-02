// omadeck — interactive demo: a virtual Stream Deck driving a simulated Omarchy desktop.
// No dependencies. Works from file:// too.
"use strict";

/* ------------------------------------------------------------------ themes */
// Palettes from Omarchy's stock themes (colors.toml), plus a custom one.
// Order: bg, fg, accent, muted, red, yellow, green, blue, magenta, cyan
const VARS = ["bg", "fg", "accent", "muted", "red", "yellow", "green", "blue", "magenta", "cyan"];
const THEMES = {
  "tokyo-night": ["Tokyo Night", "dark", "#1a1b26 #a9b1d6 #7aa2f7 #414868 #f7768e #e0af68 #9ece6a #7aa2f7 #ad8ee6 #449dab"],
  "catppuccin": ["Catppuccin", "dark", "#1e1e2e #cdd6f4 #89b4fa #585b70 #f38ba8 #f9e2af #a6e3a1 #89b4fa #f5c2e7 #94e2d5"],
  "gruvbox": ["Gruvbox", "dark", "#282828 #d4be98 #7daea3 #665c54 #ea6962 #d8a657 #a9b665 #7daea3 #d3869b #89b482"],
  "nord": ["Nord", "dark", "#2e3440 #d8dee9 #81a1c1 #4c566a #bf616a #ebcb8b #a3be8c #81a1c1 #b48ead #88c0d0"],
  "everforest": ["Everforest", "dark", "#2d353b #d3c6aa #7fbbb3 #475258 #e67e80 #dbbc7f #a7c080 #7fbbb3 #d699b6 #83c092"],
  "kanagawa": ["Kanagawa", "dark", "#1f1f28 #dcd7ba #dcd7ba #54546d #c34043 #c0a36e #76946a #7e9cd8 #957fb8 #6a9589"],
  "osaka-jade": ["Osaka Jade", "dark", "#111c18 #c1c497 #509475 #53685b #ff5345 #459451 #549e6a #509475 #d2689c #2dd5b7"],
  "matte-black": ["Matte Black", "dark", "#121212 #bebebe #e68e0d #333333 #d35f5f #b91c1c #ffc107 #e68e0d #d35f5f #bebebe"],
  "hackerman": ["Hackerman", "dark", "#0b0c16 #ddf7ff #82fb9c #2d3450 #50f872 #50f7d4 #4fe88f #829dd4 #86a7df #7cf8f7"],
  "rose-pine": ["Rosé Pine Dawn", "light", "#faf4ed #575279 #56949f #cecacd #b4637a #ea9d34 #286983 #56949f #907aa9 #d7827e"],
  "i-robot": ["I, Robot", "dark", "#12161a #c8d0d4 #7fd4e4 #3b454b #e8484f #f0b44a #6fd3a8 #6fa8dc #a79bd8 #5ec8d8"],
};

/* --------------------------------------------------------------------- keys */
// The starter layout omadeck writes on first run.
const KEYS = [
  { label: "Terminal", icon: "terminal", log: "Launch xdg-terminal-exec", run: () => openWin("terminal") },
  { label: "Browser", icon: "browser", log: "Launch omarchy launch browser", run: () => openWin("newtab") },
  { label: "Files", icon: "files", log: "Launch omarchy launch nautilus", run: () => openWin("files") },
  { label: "Screenshot", icon: "shot", log: "Launch omarchy capture screenshot", run: screenshot },
  { label: "Omarchy", icon: "web", log: "Open https://omarchy.org", run: () => openWin("omarchy") },
  ...[1, 2, 3, 4, 5].map((n) => ({ big: String(n), log: `hyprctl dispatch workspace ${n}`, run: () => switchWs(n) })),
  { label: "Mute", icon: "mute", log: "Launch omarchy audio output volume mute-toggle", run: toggleMute },
  { label: "Night light", icon: "moon", log: "Launch omarchy toggle nightlight", run: toggleNight },
  { label: "Wallpaper", icon: "wall", log: "Launch omarchy theme bg next", run: nextWall },
  { label: "Ding!", icon: "ding", log: "Play /usr/share/sounds/freedesktop/stereo/complete.oga", run: ding },
  { label: "Lock", icon: "lock", log: "Launch omarchy system lock", run: lock },
];
const KEYBOARD = "qwertasdfgzxcvb";

const $ = (s, el = document) => el.querySelector(s);
const state = {
  ws: 1,
  spaces: { 1: [], 2: [], 3: [], 4: [], 5: [] },
  focused: null,
  muted: false,
  night: false,
  wall: 0,
  locked: false,
  theme: "tokyo-night",
  terms: 0,
  touched: false,
};

/* --------------------------------------------------------------------- log */
const logEl = $("#log");
function stamp() {
  return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
}
function log(msg, level = "INFO ", target = "omadeckd") {
  const line = document.createElement("div");
  const cls = level.startsWith("WARN") ? "w" : "i";
  line.innerHTML = `<span class="t">${stamp()}</span> [<span class="${cls}">${level}</span> ${target}] `;
  line.append(msg);
  logEl.append(line);
  while (logEl.children.length > 40) logEl.firstElementChild.remove();
}

/* -------------------------------------------------------------------- deck */
const deck = $("#deck");
const keyEls = KEYS.map((k, i) => {
  const b = document.createElement("button");
  b.className = "key";
  b.type = "button";
  b.setAttribute("aria-label", `Key ${i + 1}: ${k.label ?? "Workspace " + k.big}`);
  const face = k.big
    ? `<span class="big">${k.big}</span>`
    : `<svg aria-hidden="true"><use href="#i-${k.icon}"/></svg><span class="lbl">${k.label}</span>`;
  b.innerHTML = `<span class="face">${face}</span><span class="hint" aria-hidden="true">${KEYBOARD[i].toUpperCase()}</span>`;
  b.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    b.setPointerCapture?.(e.pointerId);
    press(i);
  });
  for (const ev of ["pointerup", "pointercancel", "lostpointercapture"]) b.addEventListener(ev, () => release(i));
  b.addEventListener("click", (e) => {
    // Keyboard activation (Enter/Space) arrives as a click with detail 0.
    if (e.detail === 0) {
      press(i);
      setTimeout(() => release(i), 140);
    }
  });
  deck.append(b);
  return b;
});
const mapKeys = [...document.querySelectorAll(".keymap span")];

function press(i) {
  const el = keyEls[i];
  if (el.classList.contains("pressed")) return;
  state.touched = true;
  el.classList.add("pressed");
  mapKeys[i]?.classList.add("on");
  if (state.locked) {
    log(`key ${i}: ${KEYS[i].log}`);
    unlock();
    return;
  }
  log(`key ${i}: ${KEYS[i].log}`);
  KEYS[i].run();
}
function release(i) {
  keyEls[i].classList.remove("pressed");
  mapKeys[i]?.classList.remove("on");
}

document.addEventListener("keydown", (e) => {
  if (e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.target instanceof Element && e.target.closest("input, textarea, [contenteditable]")) return;
  const i = KEYBOARD.indexOf(e.key.toLowerCase());
  if (i >= 0) {
    e.preventDefault();
    press(i);
  }
});
document.addEventListener("keyup", (e) => {
  const i = KEYBOARD.indexOf(e.key.toLowerCase());
  if (i >= 0) release(i);
});

/* ---------------------------------------------------------------- desktop */
const screen = $("#screen");
const wsRoot = $("#workspaces");
const wsList = $("#ws-list");
const wsEls = {};
for (let n = 1; n <= 5; n++) {
  const d = document.createElement("div");
  d.className = "ws" + (n === 1 ? " on" : "");
  d.dataset.n = n;
  wsRoot.append(d);
  wsEls[n] = d;
  const li = document.createElement("li");
  li.textContent = n;
  wsList.append(li);
}

function renderBar() {
  [...wsList.children].forEach((li, idx) => {
    const n = idx + 1;
    li.classList.toggle("on", n === state.ws);
    li.classList.toggle("has", state.spaces[n].length > 0);
  });
}

function switchWs(n) {
  if (n === state.ws) return;
  const dir = n > state.ws ? 1 : -1;
  const from = wsEls[state.ws];
  const to = wsEls[n];
  to.style.setProperty("--off", `${dir * 6}%`);
  from.style.setProperty("--off", `${-dir * 6}%`);
  void to.offsetWidth;
  from.classList.remove("on");
  to.classList.add("on");
  state.ws = n;
  renderBar();
  layout(n);
}

/* Hyprland's dwindle layout: each new window splits the last one, alternating by aspect. */
function layout(n = state.ws) {
  const list = state.spaces[n];
  const W = wsRoot.clientWidth;
  const H = wsRoot.clientHeight;
  const g = Math.max(4, W * 0.011);
  let x = g, y = g, w = W - 2 * g, h = H - 2 * g;
  list.forEach((el, i) => {
    let r;
    if (i === list.length - 1) {
      r = [x, y, w, h];
    } else if (w >= h) {
      const half = (w - g) / 2;
      r = [x, y, half, h];
      x += half + g;
      w = half;
    } else {
      const half = (h - g) / 2;
      r = [x, y, w, half];
      y += half + g;
      h = half;
    }
    Object.assign(el.style, { left: `${r[0]}px`, top: `${r[1]}px`, width: `${r[2]}px`, height: `${r[3]}px` });
  });
}
new ResizeObserver(() => layout()).observe(wsRoot);

function focusWin(el) {
  state.focused?.classList.remove("focus");
  state.focused = el;
  el?.classList.add("focus");
}

function openWin(kind, lines) {
  const list = state.spaces[state.ws];
  if (list.length >= 4) closeWin(list[0], true);
  const el = document.createElement("div");
  el.className = "win";
  el.innerHTML = WINDOWS[kind]();
  const x = document.createElement("button");
  x.className = "win-x";
  x.type = "button";
  x.title = "Close (killactive)";
  x.textContent = "×";
  x.addEventListener("click", (e) => {
    e.stopPropagation();
    closeWin(el);
  });
  el.append(x);
  el.addEventListener("pointerdown", () => focusWin(el));
  list.push(el);
  layout();
  wsEls[state.ws].append(el);
  focusWin(el);
  renderBar();
  el._after?.();
  if (kind === "terminal") typeTerminal(el, lines);
}

function closeWin(el, quiet) {
  for (const n in state.spaces) {
    const list = state.spaces[n];
    const idx = list.indexOf(el);
    if (idx < 0) continue;
    list.splice(idx, 1);
    el.classList.add("closing");
    setTimeout(() => el.remove(), 260);
    layout(+n);
    if (state.focused === el) focusWin(list.at(-1) ?? null);
    if (!quiet) log("hl.dsp.window.close()", "INFO ", "hyprland");
  }
  renderBar();
}

/* ---------------------------------------------------------------- windows */
const folder = `<svg viewBox="0 0 64 64"><use href="#i-files"/></svg>`;
const WINDOWS = {
  terminal: () => `<div class="term"></div>`,
  newtab: () => `
    <div class="browser">
      <div class="tabs"><span class="tab">New Tab</span></div>
      <div class="urlbar">Search or type a URL</div>
      <div class="newtab">
        <div class="search">Search the web…</div>
        <div class="tiles"><i>gh</i><i>yt</i><i>ω</i><i>hn</i><i>r/</i></div>
      </div>
    </div>`,
  omarchy: () => `
    <div class="browser">
      <div class="tabs"><span class="tab">Omarchy</span></div>
      <div class="urlbar">https://<b>omarchy.org</b></div>
      <div class="page">
        <h4>omarchy</h4>
        <p>Beautiful, modern &amp; opinionated Linux.</p>
        <span class="pill">Download</span><span class="pill ghost">Manual</span>
        <div class="sk" style="width:92%;margin-top:2cqi"></div><div class="sk" style="width:84%"></div>
        <div class="sk" style="width:88%"></div><div class="sk" style="width:60%"></div>
      </div>
    </div>`,
  files: () => `
    <div class="files">
      <aside><span class="on">Home</span><span>Recent</span><span>Starred</span><span>Trash</span><span>Work</span></aside>
      <div class="grid">
        ${["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos", "Work"].map((f) => `<figure>${folder}<figcaption>${f}</figcaption></figure>`).join("")}
      </div>
    </div>`,
};

const DECK_ART = [
  "╭───────────────╮",
  "│ ▣  ▣  ▣  ▣  ▣ │",
  "│ ▣  ▣  ▣  ▣  ▣ │",
  "│ ▣  ▣  ▣  ▣  ▣ │",
  "╰──────┬─┬──────╯",
  "       └─┘",
];

function prompt(cmd) {
  return `<span class="p">➜</span> <span class="a">~</span> ${cmd}`;
}

function terminalScript() {
  const t = THEMES[state.theme];
  const variant = state.terms++ % 3;
  if (variant === 0) {
    const sw = VARS.slice(4).concat("accent", "fg").map((v) => `<i style="background:var(--${v})"></i>`).join("");
    const info = [
      `<span class="a">raymond</span>@<span class="a">omarchy</span>`,
      `<span class="d">─────────────────</span>`,
      `<span class="m">OS</span>     Omarchy (Arch Linux)`,
      `<span class="m">WM</span>     Hyprland`,
      `<span class="m">Theme</span>  ${t[0]}`,
      `<span class="m">Font</span>   JetBrainsMono Nerd Font`,
      `<span class="m">Deck</span>   Stream Deck (V2) · omadeckd`,
      `<div class="swatches">${sw}</div>`,
    ];
    return [
      prompt("fastfetch"),
      `<div class="ff"><div class="logo">${DECK_ART.join("\n")}</div><div>${info.join("\n")}</div></div>`,
    ];
  }
  if (variant === 1) {
    return [
      prompt("omadeckd list"),
      "Stream Deck (V2)  serial=CL24K1A01337  5x3 keys  72px",
      prompt("systemctl --user is-active omadeckd"),
      `<span class="y">active</span>`,
      prompt("ps -o rss=,%cpu= -C omadeckd"),
      " 8680  0.0",
    ];
  }
  return [
    prompt("head -9 ~/.config/omadeck/config.toml"),
    `<span class="d"># omadeck configuration</span>`,
    `<span class="a">brightness</span> = <span class="y">70</span>`,
    "",
    `<span class="m">[style]</span>`,
    `<span class="a">follow_theme</span> = <span class="y">true</span>`,
    "",
    `<span class="m">[[key]]</span>`,
    `<span class="a">index</span> = <span class="y">0</span>`,
    `<span class="a">label</span> = <span class="p">"Terminal"</span>`,
  ];
}

function typeTerminal(win, lines) {
  lines ??= terminalScript();
  const term = $(".term", win);
  let i = 0;
  const step = () => {
    if (!term.isConnected) return;
    $(".cur", term)?.remove();
    if (i < lines.length) {
      term.insertAdjacentHTML("beforeend", (i ? "\n" : "") + lines[i++]);
      term.insertAdjacentHTML("beforeend", `<span class="cur"></span>`);
      setTimeout(step, i === 1 ? 260 : 70);
    } else {
      term.insertAdjacentHTML("beforeend", "\n" + prompt(`<span class="cur"></span>`));
    }
  };
  step();
}

/* ----------------------------------------------------------- system bits */
const toastsEl = $("#toasts");
function toast(title, body) {
  const t = document.createElement("div");
  t.className = "toast";
  t.innerHTML = `<b></b><span></span>`;
  t.firstChild.textContent = title;
  t.lastChild.textContent = body;
  toastsEl.prepend(t);
  while (toastsEl.children.length > 3) toastsEl.lastElementChild.remove();
  setTimeout(() => {
    t.classList.add("out");
    setTimeout(() => t.remove(), 320);
  }, 3200);
}

function screenshot() {
  const f = $("#flash");
  f.classList.remove("go");
  void f.offsetWidth;
  f.classList.add("go");
  const now = new Date();
  const p = (n) => String(n).padStart(2, "0");
  const name = `screenshot-${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}_${p(now.getHours())}-${p(now.getMinutes())}-${p(now.getSeconds())}.png`;
  setTimeout(() => toast("Screenshot saved", `~/Pictures/${name}`), 250);
  shutter();
}

const osd = $("#osd");
let osdTimer;
function toggleMute() {
  state.muted = !state.muted;
  osd.classList.toggle("muted", state.muted);
  $("use", osd).setAttribute("href", state.muted ? "#i-mute" : "#i-sound");
  $("#bi-vol use").setAttribute("href", state.muted ? "#i-mute" : "#i-sound");
  osd.classList.add("show");
  clearTimeout(osdTimer);
  osdTimer = setTimeout(() => osd.classList.remove("show"), 1300);
}

function toggleNight() {
  state.night = !state.night;
  screen.classList.toggle("night-on", state.night);
  toast("Nightlight", state.night ? "Screen temperature set to 4000K" : "Screen temperature set to 6000K");
}

function nextWall() {
  state.wall = (state.wall + 1) % 4;
  $("#wallpaper").dataset.w = state.wall;
}

/* Sounds are synthesised with WebAudio — no audio files to ship. */
let actx;
function audio() {
  actx ??= new (window.AudioContext || window.webkitAudioContext)();
  if (actx.state === "suspended") actx.resume();
  return actx;
}
function tone(freq, at, dur, gain = 0.18, type = "sine") {
  const ctx = audio();
  const o = ctx.createOscillator();
  const g = ctx.createGain();
  o.type = type;
  o.frequency.value = freq;
  g.gain.setValueAtTime(0, ctx.currentTime + at);
  g.gain.linearRampToValueAtTime(gain, ctx.currentTime + at + 0.01);
  g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + at + dur);
  o.connect(g).connect(ctx.destination);
  o.start(ctx.currentTime + at);
  o.stop(ctx.currentTime + at + dur + 0.05);
}
function ding() {
  if (state.muted) {
    log("pw-play: output is muted", "WARN ", "omadeck_core::action");
    toast("Ding!", "…but the output is muted");
    return;
  }
  tone(1318.5, 0, 0.6);
  tone(1760, 0.12, 0.9);
  tone(2637, 0.12, 0.5, 0.05);
}
function shutter() {
  if (state.muted) return;
  try {
    const ctx = audio();
    const len = ctx.sampleRate * 0.06;
    const buf = ctx.createBuffer(1, len, ctx.sampleRate);
    const d = buf.getChannelData(0);
    for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / len) ** 2;
    const src = ctx.createBufferSource();
    const g = ctx.createGain();
    g.gain.value = 0.25;
    src.buffer = buf;
    src.connect(g).connect(ctx.destination);
    src.start();
  } catch {
    /* audio unavailable */
  }
}

const lockEl = $("#lock");
function lock() {
  state.locked = true;
  updateClock();
  lockEl.hidden = false;
}
function unlock() {
  state.locked = false;
  lockEl.hidden = true;
}
lockEl.addEventListener("click", unlock);

function updateClock() {
  const now = new Date();
  const time = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  $("#clock").textContent = `${now.toLocaleDateString([], { weekday: "short" })} ${time}`;
  $("#lock-time").textContent = time;
  $("#lock-date").textContent = now.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });
}
updateClock();
setInterval(updateClock, 15000);

/* ------------------------------------------------------------------ theme */
const themesEl = $("#themes");
function setTheme(id, quiet) {
  const t = THEMES[id];
  if (!t) return;
  state.theme = id;
  const root = document.documentElement;
  t[2].split(" ").forEach((c, i) => root.style.setProperty(`--${VARS[i]}`, c));
  root.dataset.mode = t[1];
  root.dataset.theme = id;
  $('meta[name="theme-color"]').setAttribute("content", t[2].split(" ")[0]);
  for (const chip of themesEl.children) chip.setAttribute("aria-checked", String(chip.dataset.id === id));
  try {
    localStorage.setItem("omadeck-theme", id);
  } catch {
    /* storage unavailable */
  }
  if (!quiet) log(`reloaded config (theme: ${id})`);
}
for (const [id, [name, , colors]] of Object.entries(THEMES)) {
  const c = colors.split(" ");
  const chip = document.createElement("button");
  chip.type = "button";
  chip.className = "theme-chip";
  chip.dataset.id = id;
  chip.setAttribute("role", "radio");
  chip.innerHTML = `<span class="sw"><i style="background:${c[0]}"></i><i style="background:${c[2]}"></i><i style="background:${c[8]}"></i></span>${name}`;
  chip.addEventListener("click", () => setTheme(id));
  themesEl.append(chip);
}

/* --------------------------------------------------------------- copy btns */
for (const btn of document.querySelectorAll(".copy")) {
  btn.addEventListener("click", async () => {
    const text = btn.previousElementSibling.textContent;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = Object.assign(document.createElement("textarea"), { value: text });
      document.body.append(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
    btn.classList.add("ok");
    $("use", btn).setAttribute("href", "#i-check");
    setTimeout(() => {
      btn.classList.remove("ok");
      $("use", btn).setAttribute("href", "#i-copy");
    }, 1400);
  });
}

/* ------------------------------------------------------------------- boot */
let saved = null;
try {
  saved = localStorage.getItem("omadeck-theme");
} catch {
  /* storage unavailable */
}
setTheme(saved && THEMES[saved] ? saved : "tokyo-night", true);
log("omadeckd 0.1.0 ready; config ~/.config/omadeck/config.toml");
log(`theme: ${state.theme}`);
log("connected Stream Deck (V2) (serial CL24K1A01337, firmware 1.03.000)");

// A welcome terminal on workspace 1.
openWin("terminal", [
  prompt("omadeck --hello"),
  `<span class="a">omadeck</span> is driving the Stream Deck on the right.`,
  `Press a key — or use <span class="y">Q W E R T / A S D F G / Z X C V B</span>.`,
]);
renderBar();

// Attract mode: a light wave across the keys once the deck scrolls into view.
const io = new IntersectionObserver(
  (entries) => {
    if (!entries[0].isIntersecting) return;
    io.disconnect();
    keyEls.forEach((k, i) => {
      const d = ((i % 5) + Math.floor(i / 5)) * 70;
      setTimeout(() => !state.touched && k.classList.add("lit"), 400 + d);
      setTimeout(() => k.classList.remove("lit"), 650 + d);
    });
  },
  { threshold: 0.6 },
);
io.observe(deck);
