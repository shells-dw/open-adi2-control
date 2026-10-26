'use strict';
/* Open ADI-2 Control – the window. All unit traffic goes through the Rust side (invoke/events). */
(() => {
  const T = window.__TAURI__;
  const invoke = T.core.invoke;
  const listen = T.event.listen;
  const $ = (s, r = document) => r.querySelector(s);

  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === null || v === undefined || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
      else if (k === 'text') el.textContent = v;
      else if (k === 'value') el.value = v;
      else if (k === 'checked' || k === 'disabled' || k === 'selected') el[k] = !!v;
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const kid of kids.flat(Infinity)) {
      if (kid === null || kid === undefined || kid === false) continue;
      el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
    }
    return el;
  }
  const svgNS = 'http://www.w3.org/2000/svg';
  function s(tag, attrs) {
    const el = document.createElementNS(svgNS, tag);
    for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, v);
    return el;
  }

  // ---------------------------------------------------------------- state
  const S = {
    schema: null,
    device: {},
    p: new Map(),
    setups: [],
    presets: [],
    status: null,
    config: null,
    lastOut: 'line',
    page: 'line',
    version: '',
    update: null, // update check: { checking, checkedAt, latest, newer, offer, error, install, appimage }
    updDismissed: null, // version whose bar was closed in this session
    editor: null, // EQ preset editor: { n, name, dual, L, R }
    clip: null,
    side: {}, // per channel: 'L' | 'R' being edited
    ab: null, // A/B compare after Listen: { ch, before: [[key, value]], after: [[key, value]], on: 'A' | 'B' }
    hp: null, // headphone EQ tab state (hp.js)
  };
  const SEC = {};
  let CH = {};
  const DEBUG = false;

  const key = (a, i) => `${a}.${i}`;
  const unit = () => (S.config && (S.config.units[S.config.active] || S.config.units[0])) || { mode: 'usb' };
  const model = () => S.device.model || (unit().mode === 'demo' ? unit().demo_model : 'pro');
  /** The unit is connected and read out. */
  const online = () => !!(S.device.connected && S.device.answering && S.device.ready);
  /** …and takes changes (an ADI-2-Pi without its key shows but doesn't take). */
  const usable = () => online() && !S.device.read_only;
  /** Before a change: in view-only mode ask for the key instead. */
  function guard() {
    if (online() && S.device.read_only) { askKey(); return false; }
    return usable();
  }
  const has = (k) => S.p.has(k);
  const get = (k) => S.p.get(k);
  const avail = (x) => !((x.only && !x.only.includes(model())) || (x.not && x.not.includes(model())));
  const chName = (c) => (model() === 'dac' && c.dacName ? c.dacName : c.name);
  const chShort = (c) => (model() === 'dac' ? { line: 'Line Out', ph12: 'Phones', ph34: 'IEM' }[c.key] || c.short : { in: 'Line In', line: 'Line Out' }[c.key] || c.short);
  const outputs = () => S.schema.channels.filter((c) => c.kind === 'output');

  /** Show a parameter: the model has it, and (once read out) the unit reports it. */
  function shows(k, p, c) {
    if (!avail(p) || (c && !avail(c)) || (p.lineOnly && c && !c.line)) return false;
    return S.device.ready ? has(k) : true;
  }

  function namesOf(p, c) {
    let n = p.names;
    if (!n) return null;
    if (Array.isArray(n)) return n;
    if (n.line || n.phones) n = c && c.line ? n.line : n.phones;
    if (Array.isArray(n)) return n;
    return n[model()] || n.pro || Object.values(n)[0];
  }

  const isHz = (p) => p.unit === 'Hz';
  const disp = (p, raw) => (raw === undefined ? undefined : isHz(p) ? raw : raw * (p.scale || 1));
  const toRaw = (p, d) => (isHz(p) ? Math.round(d) : Math.round(d / (p.scale || 1)));
  function fmtHz(f) {
    if (f >= 1000) {
      const k = f / 1000;
      return `${k >= 10 ? k.toFixed(k % 1 ? 1 : 0) : k.toFixed(2).replace(/0$/, '').replace(/\.0$/, '')}k`;
    }
    return String(Math.round(f));
  }
  function fmtVal(p, d) {
    if (d === undefined || d === null || Number.isNaN(d)) return '–';
    if (isHz(p)) return fmtHz(d);
    const s = d.toFixed(p.decimals ?? 0);
    const signed = p.unit === 'dB' && p.min < 0 && d > 0 ? `+${s}` : s;
    return signed.replace('-', '−');
  }

  // ---------------------------------------------------------------- live updates
  const subs = new Map(); // key → Set(fn)
  const pageSubs = new Set(); // presets/setups/status/config
  function sub(k, fn) {
    if (!subs.has(k)) subs.set(k, new Set());
    subs.get(k).add(fn);
  }
  function notify(keys) {
    const fns = new Set();
    for (const k of keys) for (const fn of subs.get(k) || []) fns.add(fn);
    for (const fn of fns) fn();
  }

  const pendingSets = new Map();
  let setFrame = 0;
  function setRaw(k, v) {
    if (!usable()) {
      if (online() && S.device.read_only) { notify([k]); askKey(); }
      return;
    }
    S.p.set(k, v);
    notify([k]);
    pendingSets.set(k, v);
    if (!setFrame) setFrame = requestAnimationFrame(flushSets);
  }
  function flushSets() {
    setFrame = 0;
    const items = [...pendingSets.entries()];
    pendingSets.clear();
    if (items.length) invoke('set', { items }).catch((e) => toast(e, true));
  }

  let toastTimer = null;
  function toast(msg, bad) {
    document.querySelectorAll('.toast').forEach((t) => t.remove());
    const t = h('div', { class: `toast${bad ? ' bad' : ''}` }, String(msg));
    document.body.append(t);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => t.remove(), bad ? 5000 : 2200);
  }

  // ---------------------------------------------------------------- small controls
  function sw(on, onclick, label, disabled) {
    const b = h('button', { class: 'sw', role: 'switch', 'aria-checked': on ? 'true' : 'false', 'aria-label': label || '', disabled, onclick });
    return b;
  }

  /** Switch bound to a toggle parameter. */
  function swKey(k, label) {
    const b = sw(!!get(k), () => setRaw(k, get(k) ? 0 : 1), label);
    const up = () => b.setAttribute('aria-checked', get(k) ? 'true' : 'false');
    sub(k, up);
    return b;
  }

  /** Button lit while a toggle parameter is on. */
  function btnKey(k, label, color = '') {
    const b = h('button', { class: 'btn big', onclick: () => setRaw(k, get(k) ? 0 : 1) }, label);
    const up = () => { b.className = `btn big${get(k) ? ` on ${color}` : ''}`; b.disabled = !has(k); };
    sub(k, up);
    up();
    return b;
  }

  function selectKey(k, p, c, extra) {
    const names = (extra && extra.names) || namesOf(p, c) || [];
    const hide = p.hide || [];
    const sel = h('select', { 'aria-label': p.label, disabled: p.readonly, onchange: () => setRaw(k, Number(sel.value)) },
      names.map((n, i) => (hide.includes(i) ? null : h('option', { value: i, text: n }))));
    const up = () => {
      const v = get(k);
      if (v !== undefined && ![...sel.options].some((o) => Number(o.value) === v)) sel.append(h('option', { value: v, text: `(${v})` }));
      sel.value = v === undefined ? '' : String(v);
    };
    sub(k, up);
    up();
    return sel;
  }

  function field(label, ctl) {
    return h('div', { class: 'field' }, h('label', null, label), ctl);
  }
  function swField(label, ctl) {
    return h('label', { class: 'swl' }, ctl, h('span', null, label));
  }

  // ---------------------------------------------------------------- knob
  const ARC = 270;
  function polar(cx, cy, r, deg) {
    const a = ((deg - 90) * Math.PI) / 180;
    return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
  }
  function arcPath(cx, cy, r, a0, a1) {
    if (Math.abs(a1 - a0) < 0.01) a1 = a0 + 0.01;
    const [x0, y0] = polar(cx, cy, r, a0), [x1, y1] = polar(cx, cy, r, a1);
    const large = Math.abs(a1 - a0) > 180 ? 1 : 0;
    const sweep = a1 > a0 ? 1 : 0;
    return `M${x0.toFixed(2)} ${y0.toFixed(2)} A${r} ${r} 0 ${large} ${sweep} ${x1.toFixed(2)} ${y1.toFixed(2)}`;
  }

  /**
   * Knob in display units.
   * o: { label, color, min, max, step, log, fmt(v), get(), set(v), disabled(), size, range (px per full
   * turn), unipolar (arc from the minimum, not from 0), volume (wheel setting "volume only" applies), def (double-click) }
   */
  function knob(o) {
    const size = o.size || 44;
    const sw2 = Math.max(3, size / 11);
    const r = size / 2 - sw2;
    const c = size / 2;
    const svg = s('svg', { class: 'knob', width: size, height: size, viewBox: `0 0 ${size} ${size}`, tabindex: 0, role: 'slider', 'aria-label': o.label || '' });
    const trk = s('path', { class: 'trk', d: arcPath(c, c, r, -ARC / 2, ARC / 2), 'stroke-width': sw2 });
    const val = s('path', { class: 'val', stroke: o.color || 'var(--accent)', 'stroke-width': sw2 });
    const face = s('circle', { class: 'face', cx: c, cy: c, r: r - sw2 * 0.9 });
    const tick = s('line', { class: 'tick', stroke: o.color || 'var(--accent)', 'stroke-width': Math.max(2, size / 22) });
    svg.append(trk, val, face, tick);
    const vEl = h('div', { class: 'knob-v' });
    const box = h('div', { class: 'knob-box' }, o.label ? h('div', { class: 'knob-l' }, o.label) : null, svg, vEl);
    const lo = o.min, hi = o.max;
    const toPos = (v) => (o.log ? Math.log(v / lo) / Math.log(hi / lo) : (v - lo) / (hi - lo));
    const fromPos = (t) => (o.log ? lo * Math.pow(hi / lo, t) : lo + t * (hi - lo));
    const snap = (v) => {
      v = Math.min(hi, Math.max(lo, v));
      if (o.log) {
        const st = v < 100 ? 1 : v < 1000 ? 5 : v < 5000 ? 10 : 100;
        return Math.round(v / st) * st;
      }
      const st = o.step || 1;
      return Math.round((v - lo) / st) * st + lo;
    };
    let cur;
    box.update = () => {
      const v = o.get();
      cur = v;
      const disabled = v === undefined || o.disabled?.();
      svg.classList.toggle('disabled', !!disabled);
      if (v === undefined) { vEl.textContent = '–'; val.setAttribute('d', ''); return; }
      const t = Math.min(1, Math.max(0, toPos(v)));
      const a = -ARC / 2 + t * ARC;
      const zero = !o.log && !o.unipolar && lo < 0 && hi > 0 ? -ARC / 2 + toPos(0) * ARC : -ARC / 2;
      val.setAttribute('d', arcPath(c, c, r, Math.min(zero, a), Math.max(zero, a)));
      const [x1, y1] = polar(c, c, r - sw2 * 2.2, a), [x0, y0] = polar(c, c, (r - sw2) * 0.35, a);
      tick.setAttribute('x1', x0); tick.setAttribute('y1', y0); tick.setAttribute('x2', x1); tick.setAttribute('y2', y1);
      vEl.textContent = (o.fmt || String)(v);
    };
    const commit = (v) => {
      v = snap(v);
      if (v === cur) return;
      cur = v;
      o.set(v);
    };
    let drag = null;
    svg.addEventListener('pointerdown', (e) => {
      if (cur === undefined || o.disabled?.()) return;
      svg.setPointerCapture(e.pointerId);
      drag = { y: e.clientY, t: toPos(cur) };
      e.preventDefault();
    });
    svg.addEventListener('pointermove', (e) => {
      if (!drag) return;
      const px = (o.range || 220) * (e.shiftKey ? 5 : 1);
      const t = Math.min(1, Math.max(0, drag.t + (drag.y - e.clientY) / px));
      commit(fromPos(t));
    });
    svg.addEventListener('pointerup', () => { drag = null; });
    svg.addEventListener('pointercancel', () => { drag = null; });
    const stepBy = (dir, fine) => {
      if (cur === undefined || o.disabled?.()) return;
      if (o.log) commit(cur * Math.pow(fine ? 1.01 : 1.06, dir));
      else commit(cur + dir * (o.step || 1) * (fine ? 1 : o.coarse || 1));
    };
    svg.addEventListener('wheel', (e) => {
      const w = (S.config && S.config.ui.wheel) || 'all';
      if (w === 'off' || (w === 'volume' && !o.volume)) return;
      e.preventDefault();
      stepBy(e.deltaY < 0 ? 1 : -1, e.shiftKey);
    }, { passive: false });
    svg.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowUp' || e.key === 'ArrowRight') { stepBy(1, e.shiftKey); e.preventDefault(); }
      if (e.key === 'ArrowDown' || e.key === 'ArrowLeft') { stepBy(-1, e.shiftKey); e.preventDefault(); }
    });
    svg.addEventListener('dblclick', () => { if (o.def !== undefined && cur !== undefined) commit(o.def); });
    vEl.addEventListener('dblclick', () => {
      if (cur === undefined || o.disabled?.()) return;
      const inp = h('input', { class: 'knob-edit', type: 'text', value: o.log ? String(cur) : String(cur) });
      vEl.replaceChildren(inp);
      inp.focus();
      inp.select();
      const done = (ok) => {
        if (ok) {
          let t = inp.value.trim().replace('−', '-').replace(',', '.');
          let m = 1;
          if (/k$/i.test(t)) { m = 1000; t = t.slice(0, -1); }
          const n = Number(t) * m;
          if (Number.isFinite(n)) commit(n);
        }
        box.update();
      };
      inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') done(true); if (e.key === 'Escape') done(false); });
      inp.addEventListener('blur', () => done(true));
    });
    box.update();
    return box;
  }

  /** Knob bound to a level parameter. */
  function knobKey(k, p, extra = {}) {
    const kb = knob({
      label: extra.label ?? p.label,
      color: extra.color,
      min: p.min, max: p.max, step: p.step || (p.scale || 1), log: !!p.log,
      size: extra.size,
      range: extra.range,
      unipolar: extra.unipolar,
      def: extra.def,
      volume: extra.volume,
      fmt: (v) => fmtVal(p, v) + (extra.unit !== undefined ? extra.unit : ''),
      get: () => disp(p, get(k)),
      set: (v) => setRaw(k, toRaw(p, v)),
      disabled: extra.disabled,
    });
    sub(k, kb.update);
    for (const d of extra.deps || []) sub(d, kb.update);
    return kb;
  }

  // ---------------------------------------------------------------- dialogs
  function dialog(title, body, actions) {
    const d = $('#dlg');
    d.replaceChildren(h('h3', null, title), body, h('div', { class: 'actions' }, actions));
    d.showModal();
    return d;
  }
  function confirmDlg(title, text, okText = 'OK') {
    return new Promise((resolve) => {
      const d = dialog(title, h('p', { class: 'hint', style: { maxWidth: '420px' } }, text), [
        h('button', { class: 'btn', onclick: () => { d.close(); resolve(false); } }, 'Cancel'),
        h('button', { class: 'btn primary', onclick: () => { d.close(); resolve(true); } }, okText),
      ]);
    });
  }
  /** View-only ADI-2-Pi: the key that allows changes. */
  function askKey() {
    const d0 = $('#dlg');
    if (d0.open) return;
    const u = unit();
    const inp = h('input', { type: 'password', placeholder: 'Access key', autocomplete: 'off', style: { width: '100%' }, 'aria-label': 'Access key' });
    const d = dialog(`Access key for ${u.name || u.pi_address || 'the ADI-2-Pi'}`, h('div', { style: { maxWidth: '400px' } },
      h('p', { class: 'hint', style: { margin: '0 0 8px' } }, 'This ADI-2-Pi shows its state to everyone but takes changes only with its access key (set on its System tab).'),
      inp), [
      h('button', { class: 'btn', onclick: () => d.close() }, 'Keep viewing'),
      h('button', { class: 'btn primary', onclick: () => { d.close(); saveUnit({ pi_key: inp.value.trim() }); } }, 'Connect'),
    ]);
    // (preventDefault: otherwise the same Enter reaches the element focused after close)
    inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); d.close(); saveUnit({ pi_key: inp.value.trim() }); } });
    setTimeout(() => inp.focus(), 30);
  }

  /** Load a setup after confirmation (window buttons, tray, shortcuts). */
  async function confirmSetup(n) {
    if (!guard()) return;
    const name = S.setups[n - 1] || '';
    const ok = await confirmDlg(`Load setup ${n}?`, `${name ? `"${name}"` : `Setup ${n}`} replaces the unit's current settings, volume included. EQ presets are not affected.`, 'Load');
    if (!ok) return;
    invoke('setup', { action: 'load', n }).then(() => toast(`Loading setup ${n}`)).catch((e) => toast(e, true));
  }

  function askName(title, value, okText = 'Save') {
    return new Promise((resolve) => {
      const inp = h('input', { type: 'text', value: value || '', maxlength: 14, style: { width: '100%' } });
      const d = dialog(title, h('div', { class: 'field' }, h('label', null, 'Name (up to 14 characters)'), inp), [
        h('button', { class: 'btn', onclick: () => { d.close(); resolve(null); } }, 'Cancel'),
        h('button', { class: 'btn primary', onclick: () => { d.close(); resolve(inp.value.trim()); } }, okText),
      ]);
      inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); d.close(); resolve(inp.value.trim()); } });
      setTimeout(() => inp.focus(), 30);
    });
  }

  // ---------------------------------------------------------------- EQ panel
  const BAND_COLORS = { B: 'var(--b-bass)', 1: 'var(--b1)', 2: 'var(--b2)', 3: 'var(--b3)', 4: 'var(--b4)', 5: 'var(--b5)', T: 'var(--b-treble)' };
  const CSSC = {};
  const cssColor = (v) => (CSSC[v] ||= getComputedStyle(document.documentElement).getPropertyValue(v.slice(4, -1)).trim() || '#ccc');
  const eqParam = (idx) => SEC.eqByIdx[idx];

  /** Data source of a channel's live EQ. */
  function liveEq(c) {
    const dualKey = key(c.addr, c.kind === 'input' ? SEC.input.indualeq.idx : SEC.output.dualeq.idx);
    const addr = (side, idx) => (side === 'R' && idx >= 3 && idx <= 19 ? c.eqR : c.eq);
    return {
      live: true,
      c,
      get: (side, idx) => get(key(addr(side, idx), idx)),
      set: (side, idx, v) => setRaw(key(addr(side, idx), idx), v),
      keyOf: (side, idx) => key(addr(side, idx), idx),
      dual: () => !!get(dualKey),
      dualKey,
      bt: () => !!get(key(c.eq, 20)),
      keys: () => {
        const ks = [dualKey];
        for (let i = 2; i <= 28; i++) { ks.push(key(c.eq, i)); ks.push(key(c.eqR, i)); }
        return ks;
      },
    };
  }

  /** Data source of the preset editor (nothing goes to the unit until Store). */
  function editorEq(onChange) {
    const ed = () => S.editor;
    return {
      live: false,
      get: (side, idx) => {
        const e = ed();
        if (!e) return undefined;
        const src = side === 'R' && e.dual && idx >= 3 && idx <= 19 ? e.R : e.L;
        return src[idx];
      },
      set: (side, idx, v) => {
        const e = ed();
        if (!e) return;
        if (side === 'R' && e.dual && idx >= 3 && idx <= 19) e.R[idx] = v; else e.L[idx] = v;
        onChange();
      },
      dual: () => !!(ed() && ed().dual),
      bt: () => true,
    };
  }

  function drawCurve(cv, src, side) {
    const dpr = window.devicePixelRatio || 1;
    const W = cv.clientWidth, H = cv.clientHeight;
    if (!W || !H) return;
    if (cv.width !== Math.round(W * dpr)) { cv.width = Math.round(W * dpr); cv.height = Math.round(H * dpr); }
    const g = cv.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, W, H);
    const F0 = 10, F1 = 22000, DB = 15;
    const x = (f) => (Math.log10(f / F0) / Math.log10(F1 / F0)) * W;
    const y = (db) => H / 2 - (db / DB) * (H / 2 - 8);
    cv._map = { x, y, W, H, F0, F1, DB };
    g.font = '10px system-ui, sans-serif';
    g.lineWidth = 1;
    for (const f of [20, 30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000]) {
      const major = [100, 1000, 10000].includes(f);
      g.strokeStyle = major ? cssColor('var(--grid-major)') : cssColor('var(--grid)');
      g.beginPath(); g.moveTo(Math.round(x(f)) + 0.5, 0); g.lineTo(Math.round(x(f)) + 0.5, H); g.stroke();
    }
    g.fillStyle = cssColor('var(--faint)');
    for (const [f, t] of [[20, '20'], [100, '100'], [1000, '1k'], [10000, '10k']]) g.fillText(t, x(f) + 3, H - 4);
    for (const db of [-12, -6, 0, 6, 12]) {
      g.strokeStyle = db === 0 ? cssColor('var(--grid-zero)') : cssColor('var(--grid)');
      g.beginPath(); g.moveTo(0, Math.round(y(db)) + 0.5); g.lineTo(W, Math.round(y(db)) + 0.5); g.stroke();
      g.fillText(db > 0 ? `+${db}` : String(db), 4, y(db) - 3);
    }
    const sides = src.dual() ? ['L', 'R'] : ['L'];
    const withBT = src.bt();
    for (const sd of sides) {
      const fl = EQ.filters((i) => src.get(sd, i), withBT);
      g.beginPath();
      for (let px = 0; px <= W; px += 2) {
        const f = F0 * Math.pow(F1 / F0, px / W);
        const db = Math.max(-DB - 5, Math.min(DB + 5, EQ.response(fl, f)));
        if (px === 0) g.moveTo(px, y(db)); else g.lineTo(px, y(db));
      }
      g.lineWidth = 2.2;
      g.strokeStyle = sd === 'R' ? cssColor('var(--curve-r)') : cssColor('var(--accent)');
      g.setLineDash(sd === 'R' && sides.length > 1 ? [6, 4] : []);
      g.globalAlpha = sides.length > 1 && sd !== side ? 0.5 : 1;
      g.stroke();
      g.setLineDash([]);
      g.globalAlpha = 1;
    }
    // band handles of the side being edited
    const pts = [];
    const list = withBT ? [EQ.BASS, ...EQ.BANDS, EQ.TREBLE] : EQ.BANDS;
    for (const b of list) {
      const f = src.get(side, b.f), gr = src.get(side, b.g);
      if (f === undefined || gr === undefined) continue;
      const kind = EQ.kindOf(b, b.type ? src.get(side, b.type) ?? 0 : 0);
      const gain = kind === 'lowpass' || kind === 'highpass' ? 0 : gr * 0.5;
      const px = x(f), py = y(gain);
      pts.push({ b, px, py });
      g.beginPath();
      g.arc(px, py, 6, 0, Math.PI * 2);
      g.fillStyle = cssColor('var(--curve)');
      g.fill();
      g.lineWidth = 2;
      g.strokeStyle = cssColor(BAND_COLORS[b.band]);
      g.stroke();
      g.fillStyle = cssColor(BAND_COLORS[b.band]);
      g.font = 'bold 9px system-ui, sans-serif';
      g.textAlign = 'center';
      g.fillText(String(b.band), px, py - 10);
      g.textAlign = 'start';
    }
    cv._pts = pts;
  }

  function attachCurveDrag(cv, src, sideOf, redraw) {
    let drag = null;
    const hit = (e) => {
      const r = cv.getBoundingClientRect();
      const mx = e.clientX - r.left, my = e.clientY - r.top;
      let best = null, bd = 14;
      for (const p of cv._pts || []) {
        const d = Math.hypot(p.px - mx, p.py - my);
        if (d < bd) { bd = d; best = p; }
      }
      return { best, mx, my };
    };
    cv.addEventListener('pointerdown', (e) => {
      const { best } = hit(e);
      if (!best || (src.live && !usable())) return;
      cv.setPointerCapture(e.pointerId);
      drag = best.b;
      e.preventDefault();
    });
    cv.addEventListener('pointermove', (e) => {
      if (!drag) { cv.style.cursor = hit(e).best ? 'grab' : 'default'; return; }
      const { mx, my } = hit(e);
      const m = cv._map;
      const side = sideOf();
      const fp = eqParam(drag.f), gp = eqParam(drag.g);
      let f = m.F0 * Math.pow(m.F1 / m.F0, Math.min(1, Math.max(0, mx / m.W)));
      f = Math.min(fp.max, Math.max(fp.min, f));
      f = f < 100 ? Math.round(f) : f < 1000 ? Math.round(f / 5) * 5 : f < 5000 ? Math.round(f / 10) * 10 : Math.round(f / 100) * 100;
      const db = ((m.H / 2 - my) / (m.H / 2 - 8)) * m.DB;
      const graw = Math.round(Math.min(gp.max, Math.max(gp.min, db)) / 0.5);
      if (src.get(side, drag.f) !== f) src.set(side, drag.f, f);
      if (src.get(side, drag.g) !== graw) src.set(side, drag.g, graw);
      redraw();
    });
    const end = () => { drag = null; };
    cv.addEventListener('pointerup', end);
    cv.addEventListener('pointercancel', end);
    cv.addEventListener('wheel', (e) => {
      const { best } = hit(e);
      if (!best || (src.live && !usable())) return;
      e.preventDefault();
      const qp = eqParam(best.b.q);
      const side = sideOf();
      const cur = src.get(side, best.b.q) ?? 10;
      const next = Math.min(Math.round(qp.max * 10), Math.max(Math.round(qp.min * 10), cur + (e.deltaY < 0 ? 1 : -1)));
      src.set(side, best.b.q, next);
      redraw();
    }, { passive: false });
  }

  /** PEQ panel: curve, 5 bands + Bass/Treble, Dual, band types; for a live EQ or the preset editor. */
  function eqPanel(src, opts = {}) {
    const c = src.c;
    const sideKey = c ? c.key : 'editor';
    const sideOf = () => (src.dual() ? S.side[sideKey] || 'L' : 'L');
    const cv = h('canvas', { class: 'curve' });
    const redraw = () => requestAnimationFrame(() => drawCurve(cv, src, sideOf()));
    attachCurveDrag(cv, src, sideOf, redraw);
    cv.addEventListener('themechange', redraw);
    new ResizeObserver(redraw).observe(cv);

    const P = (id) => SEC.eq[id];
    const bandKnobs = (b, label, color, cls) => {
      const gp = eqParam(b.g), fp = eqParam(b.f), qp = eqParam(b.q);
      const isCut = () => b.type && ['lowpass', 'highpass'].includes(EQ.kindOf(b, src.get(sideOf(), b.type) ?? 0));
      const mk = (p, idx, extra) => {
        const kb = knob({
          label: extra.label, color, min: p.min, max: p.max, step: p.step || p.scale || 1, log: !!p.log, size: 40,
          fmt: (v) => fmtVal(p, v), def: extra.def,
          get: () => disp(p, src.get(sideOf(), idx)),
          set: (v) => { src.set(sideOf(), idx, toRaw(p, v)); redraw(); },
          disabled: extra.disabled,
        });
        return kb;
      };
      const isBT = b.band === 'B' || b.band === 'T';
      const col = h('div', { class: `band${cls ? ` ${cls}` : ''}` },
        h('h3', { style: { color } }, label),
        mk(gp, b.g, { label: 'Gain', def: 0, disabled: isCut }),
        mk(fp, b.f, { label: 'Freq' }),
        mk(qp, b.q, { label: 'Q', def: isBT ? undefined : 1 }));
      col.isBT = isBT;
      return col;
    };
    const cols = [
      bandKnobs(EQ.BASS, 'Bass', BAND_COLORS.B, 'bt'),
      ...EQ.BANDS.map((b) => bandKnobs(b, String(b.band), BAND_COLORS[b.band])),
      bandKnobs(EQ.TREBLE, 'Treble', BAND_COLORS.T, 'bt'),
    ];
    const bands = h('div', { class: 'bands' }, cols);
    // the unit reports Bass/Treble for this channel (an ADI-2 Pro EX sends none for the input)
    const hasBT = () => !src.live || !S.device.ready || has(key(c.eq, 20));
    const updateAll = () => {
      bands.classList.toggle('no-bt', !hasBT());
      btRow.hidden = !hasBT();
      for (const col of cols) {
        col.querySelectorAll('.knob-box').forEach((k) => k.update && k.update());
        if (col.isBT) col.classList.toggle('off', !src.bt());
      }
      seg.querySelectorAll('button').forEach((b) => {
        b.setAttribute('aria-pressed', sideOf() === b.value ? 'true' : 'false');
        b.disabled = !src.dual();
      });
      redraw();
    };

    // side column
    const seg = h('div', { class: 'seg' }, ['L', 'R'].map((sd) => h('button', { value: sd, onclick: () => { S.side[sideKey] = sd; updateAll(); } }, sd)));
    const typeSel = (b) => {
      const p = eqParam(b.type);
      const sel = h('select', { 'aria-label': p.label, onchange: () => { src.set(sideOf(), b.type, Number(sel.value)); updateAll(); } },
        (namesOf(p) || []).map((n, i) => h('option', { value: i, text: n })));
      sel.update = () => { const v = src.get(sideOf(), b.type); sel.value = v === undefined ? '' : String(v); };
      sel.update();
      return sel;
    };
    const t1 = typeSel(EQ.BANDS[0]), t5 = typeSel(EQ.BANDS[4]);
    const dualSw = src.live
      ? swKey(src.dualKey, 'Dual EQ')
      : sw(src.dual(), () => { S.editor.dual = !S.editor.dual; if (S.editor.dual) for (let i = 3; i <= 19; i++) S.editor.R[i] ??= S.editor.L[i]; dualSw.setAttribute('aria-checked', S.editor.dual ? 'true' : 'false'); opts.onChange && opts.onChange(); updateAll(); }, 'Dual EQ');
    const btRow = h('div', { class: 'eq-bt' },
      src.live ? swField('Bass / Treble', swKey(key(c.eq, 20), 'Bass / Treble')) : null,
      src.live && (has(key(c.eq, 27)) || !S.device.ready) ? swField('B/T load with EQ preset', swKey(key(c.eq, 27), 'Load with EQ')) : null);
    const side = h('div', { class: 'eq-side' },
      h('div', { class: 'inline' }, swField('Dual', dualSw), seg),
      btRow,
      field('Band 1', t1),
      field('Band 5', t5));
    const updateSel = () => { t1.update(); t5.update(); };

    // header
    const head = h('div', { class: 'ph' });
    if (src.live) {
      const eqKey = key(c.eq, 2);
      const presetKey = key(c.eq, 28);
      const pp = P('eqpreset');
      const sel = h('select', { 'aria-label': 'EQ Preset', onchange: () => setRaw(presetKey, Number(sel.value)) });
      const fillSel = () => {
        const names = namesOf(pp) || [];
        sel.replaceChildren(...names.map((n, i) => {
          if ((pp.hide || []).includes(i)) return null;
          let t = n;
          if (i === 0) t = 'Manual';
          else if (i === 1) t = 'Temp (changed preset)';
          else if (i >= 2 && i <= 21) {
            const pr = S.presets[i - 2];
            t = `${i - 1} · ${pr && pr.name ? pr.name : pr && pr.empty ? '(empty)' : `Preset ${i - 1}`}`;
          } else t = n.charAt(0).toUpperCase() + n.slice(1);
          return h('option', { value: i, text: t });
        }).filter(Boolean));
        const v = get(presetKey);
        sel.value = v === undefined ? '' : String(v);
      };
      fillSel();
      sub(presetKey, fillSel);
      pageSubs.add(fillSel);
      head.append(swKey(eqKey, 'EQ'), h('h2', null, opts.title || 'Parametric EQ'), h('div', { class: 'grow' }),
        h('span', { class: 'lab' }, 'Preset'), sel,
        h('button', { class: 'btn', onclick: () => storeLive(src) }, 'Store'));
    } else {
      head.append(h('h2', null, opts.title || 'EQ Preset Editor'), h('div', { class: 'grow' }), ...(opts.headExtra || []));
    }

    const panel = h('section', { class: `panel${src.live ? ' needs-unit' : ''}` }, head, h('div', { class: 'curve-wrap' }, cv),
      h('div', { class: 'eq-body' }, bands, side));
    panel.refresh = () => { updateAll(); updateSel(); };
    if (src.live) for (const k of src.keys()) sub(k, () => { updateSel(); updateAll(); });
    updateAll();
    return panel;
  }

  /** Store the live EQ of an output into one of the unit's 20 presets. */
  function storeLive(src) {
    if (!guard()) return;
    const c = src.c;
    const slot = h('select', null, S.presets.map((p) => h('option', { value: p.n, text: `${p.n} · ${p.name || (p.empty ? '(empty)' : `Preset ${p.n}`)}` })));
    const cur = get(key(c.eq, 28));
    if (cur >= 2 && cur <= 21) slot.value = String(cur - 1);
    const name = h('input', { type: 'text', maxlength: 14, value: (S.presets[Number(slot.value) - 1] || {}).name || '' });
    slot.addEventListener('change', () => { name.value = (S.presets[Number(slot.value) - 1] || {}).name || ''; });
    const d = dialog('Store EQ as preset', h('div', { class: 'fields', style: { gridTemplateColumns: '1fr 1fr', minWidth: '380px' } }, field('Preset slot', slot), field('Name', name)), [
      h('button', { class: 'btn', onclick: () => d.close() }, 'Cancel'),
      h('button', { class: 'btn primary', onclick: () => {
        d.close();
        const L = {}, R = {};
        for (let i = 3; i <= 26; i++) { if (i === 20) continue; const v = src.get('L', i); if (v !== undefined) L[i] = v; }
        const dual = src.dual();
        if (dual) for (let i = 3; i <= 19; i++) { const v = src.get('R', i); if (v !== undefined) R[i] = v; }
        invoke('store_preset', { n: Number(slot.value), name: name.value.trim() || null, l: L, r: R, dual })
          .then(() => toast(`Stored as preset ${slot.value}`)).catch((e) => toast(e, true));
      } }, 'Store to device'),
    ]);
  }

  // ---------------------------------------------------------------- pages
  function pageOutput(c) {
    const O = SEC.output;
    const K = (id) => key(c.addr, O[id].idx);
    const show = (id) => O[id] && shows(K(id), O[id], c);
    const volP = O.vol;
    const vk = K('vol');

    // volume
    const limitDb = () => { const pr = S.config.volume.profiles[S.config.volume.profile] || {}; const l = (pr.limit || {})[c.key]; return l === null || l === undefined ? null : l; };
    // about 0.15 dB per pixel (Shift: finer), so a drag can't jump the volume up
    const big = knobKey(vk, volP, { label: '', size: 128, volume: true, unit: '', range: 800, unipolar: true });
    big.querySelector('.knob-v').remove();
    const vNum = h('div', { class: 'v' });
    const vWarn = h('div', { class: 'warn' });
    const upVol = () => {
      const v = disp(volP, get(vk));
      vNum.textContent = v === undefined ? '–' : fmtVal(volP, v);
      const lim = limitDb();
      vWarn.textContent = get(K('mute')) ? 'Muted' : get(K('dim')) ? 'Dim (−20 dB)' : lim !== null && v !== undefined && v >= lim ? 'Limit reached' : '';
    };
    sub(vk, upVol);
    if (show('mute')) sub(K('mute'), upVol);
    if (show('dim')) sub(K('dim'), upVol);
    upVol();
    const steps = h('div', { class: 'inline' },
      h('button', { class: 'btn', title: 'Volume down', onclick: () => step(-1) }, '−'),
      h('button', { class: 'btn', title: 'Volume up', onclick: () => step(1) }, '+'));
    function step(dir) {
      const v = get(vk);
      if (v === undefined) return;
      setRaw(vk, v + dir * Math.round((S.config.volume.step || 1) * 10));
    }

    const linkOn = () => (S.config.volume.link || []).includes(c.key);
    const linkBtn = h('button', { class: 'btn big', title: 'Volumes of linked outputs move together (changes made in this app)', onclick: () => {
      const link = new Set(S.config.volume.link || []);
      if (link.has(c.key)) link.delete(c.key); else link.add(c.key);
      saveConfig({ volume: { link: [...link] } });
    } }, 'Link');
    const upLink = () => { linkBtn.className = `btn big${linkOn() ? ' on' : ''}`; };
    upLink();
    pageSubs.add(upLink);
    const recallBtn = h('button', { class: 'btn big', title: 'Set the volume stored under Settings → Volume', onclick: () => {
      const r = S.config.volume.recall[c.key];
      if (r === undefined || r === null) { toast('No recall volume yet: set one under Settings → Volume'); return; }
      setRaw(vk, Math.round(r * 10));
    } }, 'Recall');

    const left = h('div', { class: 'vol-left' },
      show('src') ? field('Source', selectKey(K('src'), O.src, c)) : null,
      show('ref') ? field(c.line ? 'Ref Level' : 'Ref Level', h('div', { class: 'inline' }, selectKey(K('ref'), O.ref, c), show('autoref') ? swField('Auto', swKey(K('autoref'), 'Auto Ref Level')) : null)) : null);
    const right = h('div', { class: 'vol-right' }, h('div', { class: 'btn-grid' },
      recallBtn,
      show('lockvol') ? btnKey(K('lockvol'), 'Lock', 'amber') : h('span'),
      linkBtn,
      show('dim') ? btnKey(K('dim'), 'Dim', 'amber') : h('span'),
      h('span'),
      show('mute') ? btnKey(K('mute'), 'Mute', 'red') : h('span')));
    const bal = show('bal') ? (() => {
      const p = O.bal, k = K('bal');
      const r = h('input', { type: 'range', min: -100, max: 100, step: 1, 'aria-label': 'Balance', oninput: () => setRaw(k, Number(r.value)), ondblclick: () => setRaw(k, 0) });
      const t = h('span', { class: 'num', style: { minWidth: '52px', textAlign: 'right' } });
      const up = () => { const v = get(k) ?? 0; r.value = v; t.textContent = v === 0 ? '<C>' : v < 0 ? `L ${-v}` : `R ${v}`; };
      sub(k, up); up();
      return h('div', { class: 'bal' }, h('span', { class: 'lab' }, 'Balance'), r, t);
    })() : null;
    const volPanel = h('section', { class: 'panel needs-unit' },
      h('div', { class: 'vol-panel' },
        left,
        h('div', { class: 'vol-center' }, big, h('div', { class: 'vol-read' }, vNum, h('div', { class: 'u' }, 'dB'), vWarn, steps)),
        right),
      bal);

    // advanced
    const sel = (id, label) => (show(id) ? field(label || O[id].label, selectKey(K(id), O[id], c)) : null);
    const adv = h('section', { class: 'panel needs-unit' }, h('h2', null, 'Advanced settings'),
      h('div', { class: 'inline', style: { alignItems: 'flex-start', gap: '18px' } },
        show('width') ? knobKey(K('width'), O.width, { label: 'Width', def: 1 }) : null,
        h('div', { class: 'fields', style: { flex: 1 } },
          sel('xfeed', 'Crossfeed'), sel('dafilt', 'DA Filter'), sel('mono'), sel('pol', 'Polarity'),
          show('ms') ? field('M/S Processing', swKey(K('ms'), 'M/S')) : null)));
    const loud = h('section', { class: 'panel needs-unit' },
      h('div', { class: 'ph' }, show('loud') ? swKey(K('loud'), 'Loudness') : null, h('h2', null, 'Loudness')),
      h('div', { class: 'inline', style: { gap: '18px' } },
        show('ldbass') ? knobKey(K('ldbass'), O.ldbass, { label: 'Bass', def: 7 }) : null,
        show('ldtreb') ? knobKey(K('ldtreb'), O.ldtreb, { label: 'Treble', def: 7 }) : null,
        show('ldref') ? knobKey(K('ldref'), O.ldref, { label: 'Low Vol Ref', def: -30 }) : null));
    const more = h('section', { class: 'panel needs-unit' }, h('div', { class: 'fields' },
      sel('dcp', 'DC Protection'), sel('deemph', 'De-Emphasis'), sel('loop', 'Loopback'), sel('reartrs')));

    return h('div', { class: 'page' }, volPanel, eqPanel(liveEq(c)), h('div', { class: 'row2' }, adv, loud), more);
  }

  function pageInput() {
    const c = CH.in;
    const I = SEC.input;
    const K = (id) => key(c.addr, I[id].idx);
    const show = (id) => I[id] && shows(K(id), I[id], c);
    const sel = (id, label) => (show(id) ? field(label || I[id].label, selectKey(K(id), I[id], c)) : null);
    const gen = h('section', { class: 'panel needs-unit' }, h('h2', null, 'Line input'),
      h('div', { class: 'fields' },
        sel('adconv', 'Conversion'),
        show('inref') ? field('Ref Level', h('div', { class: 'inline' }, selectKey(K('inref'), I.inref, c), show('inautoref') ? swField('Auto', swKey(K('inautoref'), 'Auto')) : null)) : null));
    const adv = h('section', { class: 'panel needs-unit' }, h('h2', null, 'Advanced settings'),
      h('div', { class: 'inline', style: { alignItems: 'flex-start', gap: '18px' } },
        show('trimL') ? knobKey(K('trimL'), I.trimL, { label: 'Trim L', def: 0 }) : null,
        show('trimR') ? knobKey(K('trimR'), I.trimR, { label: 'Trim R', def: 0 }) : null,
        h('div', { class: 'fields', style: { flex: 1 } },
          sel('adfilt'), sel('dcfilt'), sel('inphase', 'Polarity'),
          show('inms') ? field('M/S Processing', swKey(K('inms'), 'M/S')) : null,
          sel('riaa', 'RIAA Mode'),
          show('riaamono') ? field('RIAA Mono Bass', swKey(K('riaamono'), 'RIAA Mono Bass')) : null)));
    return h('div', { class: 'page' }, gen, eqPanel(liveEq(c)), adv);
  }

  /** A device setting as a row: label left, control right. */
  function paramRow(p, k) {
    let ctl;
    if (p.type === 'toggle') ctl = swKey(k, p.label);
    else if (p.type === 'select' && p.readonly) {
      ctl = h('div', { class: 'num' });
      const up = () => { const n = namesOf(p) || []; ctl.textContent = n[get(k)] ?? (get(k) ?? '–'); };
      sub(k, up); up();
    } else if (p.type === 'select') {
      ctl = selectKey(k, p);
      ctl.style.width = '190px';
    } else ctl = knobKey(k, p, { label: '' });
    return opt(p.label, p.note || null, ctl);
  }

  function pageDevice() {
    const D = S.schema.params.device;
    const groups = {};
    for (const p of D) {
      const k = key(12, p.idx);
      if (!shows(k, p)) continue;
      (groups[p.group] ||= []).push(paramRow(p, k));
    }
    const titles = { mode: 'Device mode and clock', phones: 'Phones', dsd: 'DSD', display: 'Display and keys', power: 'Power', status: 'Status' };
    const panels = Object.entries(groups).filter(([g]) => g !== 'setups').map(([g, ctls]) =>
      h('section', { class: 'panel needs-unit' }, h('h2', null, titles[g] || S.schema.groups[g] || g), h('div', { class: 'opts' }, ctls)));
    return h('div', { class: 'page' }, setupsPanel(groups.setups || []), h('div', { class: 'cols' }, panels));
  }

  function setupsPanel(extra) {
    const list = h('div', { class: 'setups' });
    const draw = () => {
      list.replaceChildren(...S.setups.map((name, i) => {
        const n = i + 1;
        return h('div', { class: 'setup' },
          h('span', { class: 'n' }, n),
          h('span', { class: 'name', title: name }, name || `Setup ${n}`),
          h('button', { class: 'btn', onclick: () => confirmSetup(n) }, 'Load'),
          h('button', { class: 'btn', onclick: async () => {
            if (!guard()) return;
            const nm = await askName(`Save the current state as setup ${n}`, name, 'Save');
            if (nm === null) return;
            invoke('setup', { action: 'save', n, name: nm || null }).then(() => toast(`Saved as setup ${n}`)).catch((e) => toast(e, true));
          } }, 'Save'),
          h('button', { class: 'btn', title: 'Rename', onclick: async () => {
            if (!guard()) return;
            const nm = await askName(`Rename setup ${n}`, name, 'Rename');
            if (!nm) return;
            invoke('setup', { action: 'rename', n, name: nm }).catch((e) => toast(e, true));
          } }, '✎'));
      }));
    };
    draw();
    pageSubs.add(draw);
    const factory = h('button', { class: 'btn', onclick: async () => {
      if (!guard()) return;
      if (!(await confirmDlg('Load factory settings?', 'This changes most settings of the unit to RME\'s defaults. EQ presets stay as they are. Save the current state as a setup first if you want to keep it.', 'Load factory settings'))) return;
      invoke('setup', { action: 'factory', n: 0 }).then(() => toast('Factory settings loaded')).catch((e) => toast(e, true));
    } }, 'Load factory settings…');
    return h('section', { class: 'panel needs-unit' },
      h('div', { class: 'ph' }, h('h2', null, 'Setups'), h('div', { class: 'grow' }), factory),
      list,
      extra.length ? h('div', { class: 'opts', style: { marginTop: '10px', maxWidth: '420px' } }, extra) : null,
      h('p', { class: 'hint' }, 'A setup holds the whole state of the unit except the 20 EQ presets, which all setups share.'));
  }

  // ---- EQ presets
  function presetToEditor(n) {
    const p = S.presets[n - 1];
    const def = S.schema.eqDefaults.values;
    const L = {}, R = {};
    for (let i = 3; i <= 26; i++) if (i !== 20) L[i] = p && p.L && p.L[i] !== undefined ? p.L[i] : def[i];
    for (let i = 3; i <= 19; i++) R[i] = p && p.dual && p.R && p.R[i] !== undefined ? p.R[i] : L[i];
    S.editor = { n, name: (p && p.name) || '', dual: !!(p && p.dual), L, R, base: null };
    S.editor.base = editorSig();
  }
  const editorSig = () => JSON.stringify([S.editor.name, S.editor.dual, S.editor.L, S.editor.dual ? S.editor.R : null]);
  const editorChanged = () => S.editor && S.editor.base !== editorSig();

  function pagePresets() {
    if (!S.editor) presetToEditor(1);
    const grid = h('div', { class: 'presets' });
    let panel;
    const drawGrid = () => {
      grid.replaceChildren(...S.presets.map((p) => {
        const sel = S.editor && S.editor.n === p.n;
        return h('div', { class: `preset${sel && editorChanged() ? ' changed' : ''}`, role: 'option', 'aria-selected': sel ? 'true' : 'false', onclick: async () => {
          if (sel) return;
          if (editorChanged() && !(await confirmDlg('Discard changes?', `Preset ${S.editor.n} has changes that are not stored on the unit.`, 'Discard'))) return;
          presetToEditor(p.n);
          drawAll();
        } },
        h('span', { class: 'ring' }), h('span', { class: 'n' }, p.n),
        h('span', { class: `name${p.name ? '' : ' unset'}` }, p.name || (p.empty === null || p.empty === undefined ? '…' : 'empty')));
      }));
    };
    const nameIn = h('input', { type: 'text', maxlength: 14, placeholder: 'Name', style: { width: '150px' }, oninput: () => { S.editor.name = nameIn.value; drawGrid(); } });
    const target = h('select', { 'aria-label': 'Listen on', onchange: () => { S.ab = null; drawAb(); } }, outputs().filter((c) => shows(key(c.addr, 12), SEC.output.vol, c)).map((c) => h('option', { value: c.key, text: chName(c) })));
    target.value = S.lastOut && CH[S.lastOut] ? S.lastOut : 'line';
    const editorItems = (c) => {
      const items = [];
      for (let i = 3; i <= 19; i++) { items.push([key(c.eq, i), S.editor.L[i]]); if (S.editor.dual) items.push([key(c.eqR, i), S.editor.R[i]]); }
      for (let i = 21; i <= 26; i++) items.push([key(c.eq, i), S.editor.L[i]]);
      items.push([key(c.addr, SEC.output.dualeq.idx), S.editor.dual ? 1 : 0], [key(c.eq, 2), 1]);
      return items;
    };
    const sendItems = (items) => {
      for (const [k, v] of items) S.p.set(k, v);
      return invoke('set', { items });
    };
    const listenBtn = h('button', { class: 'btn', title: 'Send this EQ to the chosen output (its Manual EQ); nothing is stored. A/B then switches between the output\'s previous EQ and this one.', onclick: () => {
      const c = CH[target.value];
      if (!c || !guard()) return;
      const after = editorItems(c);
      // what the output had, for A/B
      if (!S.ab || S.ab.ch !== c.key) {
        const before = [];
        for (const [k] of after) if (has(k)) before.push([k, get(k)]);
        S.ab = { ch: c.key, before, after, on: 'B' };
      } else {
        S.ab.after = after;
        S.ab.on = 'B';
      }
      sendItems(after).then(() => { toast(`Listening on ${chName(c)}`); drawAb(); }).catch((e) => toast(e, true));
    } }, 'Listen');
    const abBox = h('div', { class: 'seg', title: 'A: the EQ the output had before Listen · B: this preset' },
      ['A', 'B'].map((which) => h('button', { value: which, onclick: () => {
        if (!S.ab || S.ab.ch !== target.value || !guard()) return;
        const items = which === 'A' ? S.ab.before : S.ab.after;
        S.ab.on = which;
        sendItems(items).then(drawAb).catch((e) => toast(e, true));
      } }, which)));
    const drawAb = () => {
      const active = S.ab && S.ab.ch === target.value;
      abBox.querySelectorAll('button').forEach((b) => { b.disabled = !active; b.setAttribute('aria-pressed', active && S.ab.on === b.value ? 'true' : 'false'); });
    };
    drawAb();
    const actions = [
      nameIn,
      h('span', { class: 'lab' }, 'on'), target, listenBtn, abBox,
      h('button', { class: 'btn', title: 'Copy this EQ', onclick: () => { S.clip = JSON.parse(JSON.stringify(S.editor)); toast(`Copied preset ${S.editor.n}`); } }, 'Copy'),
      h('button', { class: 'btn', title: 'Paste the copied EQ (name included) here', onclick: () => {
        if (!S.clip) { toast('Copy a preset first'); return; }
        const n = S.editor.n, base = S.editor.base;
        S.editor = { ...JSON.parse(JSON.stringify(S.clip)), n, base };
        drawAll();
      } }, 'Paste'),
      h('button', { class: 'btn', title: 'Flat: all bands 0 dB', onclick: () => {
        const def = S.schema.eqDefaults.values;
        for (let i = 3; i <= 26; i++) if (i !== 20) S.editor.L[i] = def[i];
        for (let i = 3; i <= 19; i++) S.editor.R[i] = def[i];
        drawAll();
      } }, 'Clear'),
      h('button', { class: 'btn', title: 'Back to what the unit has stored', onclick: () => { presetToEditor(S.editor.n); drawAll(); } }, 'Re-load'),
      h('button', { class: 'btn primary needs-unit', onclick: () => {
        if (!guard()) return;
        const e = S.editor;
        const L = { ...e.L }, R = e.dual ? { ...e.R } : {};
        invoke('store_preset', { n: e.n, name: e.name.trim() || null, l: L, r: R, dual: e.dual })
          .then(() => { e.base = editorSig(); drawGrid(); toast(`Stored preset ${e.n}`); }).catch((err) => toast(err, true));
      } }, 'Store'),
    ];
    const src = editorEq(() => drawGrid());
    function drawAll() {
      nameIn.value = S.editor.name;
      drawGrid();
      const np = eqPanel(src, { title: `EQ preset ${S.editor.n}`, headExtra: actions, onChange: drawGrid });
      if (panel) panel.replaceWith(np);
      panel = np;
    }
    pageSubs.add(() => { drawGrid(); });
    drawAll();
    const importBtn = h('button', { class: 'btn', title: 'Presets from an ADI-2 Remote file (.adieqpr)', onclick: () => importPresets(() => { drawAll(); }) }, 'Import…');
    const exportBtn = h('button', { class: 'btn needs-unit', title: 'All stored presets as an ADI-2 Remote file (.adieqpr)', onclick: exportPresets }, 'Export…');
    return h('div', { class: 'page' },
      h('section', { class: 'panel' }, h('div', { class: 'ph' }, h('h2', null, 'EQ presets on the unit'), h('div', { class: 'grow' }),
        h('span', { class: 'hint', style: { margin: 0 } }, 'Pick one to edit it below. Changes stay here until you press Store.'), importBtn, exportBtn), grid),
      panel);
  }

  // ---- preset files (.adieqpr, ADI-2 Remote's format)
  function exportPresets() {
    invoke('export_presets', { slots: null })
      .then((text) => invoke('save_file', { title: 'Export EQ presets', name: 'adi2-eq-presets.adieqpr', ext: ['adieqpr'], label: 'ADI-2 EQ presets', text }))
      .then((path) => { if (path) toast(`Saved ${path}`); })
      .catch((e) => toast(e, true));
  }

  /**
   * Pick a file, show its presets, store the chosen ones or open one in the editor.
   * `presets` given: skip the file dialog (the headphone EQ tab hands over a result).
   */
  async function importPresets(onEditor, presets, sourceName) {
    const fromFile = !presets;
    try {
      if (!presets) {
        const picked = await invoke('pick_file', { title: 'Import EQ presets', ext: ['adieqpr', 'xml'], label: 'ADI-2 EQ presets' });
        if (!picked) return;
        sourceName = picked[0];
        presets = await invoke('parse_presets', { text: picked[1] });
      }
    } catch (e) { toast(e, true); return; }
    const slotOpts = () => S.presets.map((p) => h('option', { value: p.n, text: `${p.n} · ${p.name || (p.empty ? '(empty)' : `Preset ${p.n}`)}` }));
    const rows = presets.map((p) => {
      const cb = h('input', { type: 'checkbox', checked: true, 'aria-label': `Import ${p.name || `preset ${p.n}`}` });
      const slot = h('select', { 'aria-label': 'Store in slot' }, slotOpts());
      slot.value = String(Math.min(20, Math.max(1, p.n)));
      const edit = h('button', { class: 'btn', title: 'Open in the editor without storing', onclick: () => {
        d.close();
        const n = Number(slot.value);
        const def = S.schema.eqDefaults.values;
        const L = {}, R = {};
        for (let i = 3; i <= 26; i++) if (i !== 20) L[i] = p.L[i] !== undefined ? p.L[i] : def[i];
        for (let i = 3; i <= 19; i++) R[i] = p.dual && p.R[i] !== undefined ? p.R[i] : L[i];
        S.editor = { n, name: p.name || '', dual: !!p.dual, L, R, base: null };
        if (S.page !== 'presets') go('presets'); else onEditor && onEditor();
      } }, 'Edit');
      return { p, cb, slot, row: h('tr', null, h('td', null, cb), h('td', null, p.name || h('i', { class: 'hint' }, 'no name'), fromFile ? h('small', { class: 'hint', style: { display: 'block', margin: 0 } }, `slot ${p.n} in the file${p.dual ? ' · dual' : ''}`) : null), h('td', null, slot), h('td', null, edit)) };
    });
    const d = dialog(fromFile ? `Import ${sourceName ? `from ${sourceName}` : 'EQ presets'}` : `Store ${sourceName ? `the EQ for ${sourceName}` : 'EQ'} as a preset`, h('div', { style: { maxWidth: '560px' } },
      h('table', { class: 'tbl' }, h('tr', null, h('td'), h('td', null, 'Preset'), h('td', null, 'Store in slot'), h('td')), rows.map((r) => r.row)),
      h('p', { class: 'hint' }, 'Storing replaces what the chosen slot holds on the unit.')), [
      h('button', { class: 'btn', onclick: () => d.close() }, 'Cancel'),
      h('button', { class: 'btn primary needs-unit', onclick: async () => {
        if (!guard()) return;
        const todo = rows.filter((r) => r.cb.checked);
        if (!todo.length) { toast('Nothing selected'); return; }
        d.close();
        let done = 0;
        for (const r of todo) {
          const p = r.p;
          try {
            await invoke('store_preset', { n: Number(r.slot.value), name: p.name || null, l: p.L, r: p.dual ? p.R : {}, dual: !!p.dual });
            done++;
          } catch (e) { toast(e, true); break; }
        }
        if (done) toast(`Stored ${done} preset${done === 1 ? '' : 's'}`);
      } }, fromFile ? 'Store selected' : 'Store'),
    ]);
  }

  // ---- settings
  function saveConfig(patch) {
    return invoke('set_config', { patch }).then((c) => { S.config = c; applyTheme(); for (const fn of pageSubs) fn(); return c; }).catch((e) => { toast(e, true); });
  }
  /** Change the active saved connection (or `idx`). */
  function saveUnit(patch, idx = S.config.active) {
    const units = S.config.units.map((u, i) => (i === idx ? { ...u, ...patch } : u));
    return saveConfig({ units, active: idx });
  }
  function saveProfiles(profiles, profile = S.config.volume.profile) {
    return saveConfig({ volume: { profiles, profile } });
  }
  const NEW_UNIT = { name: '', mode: 'usb', pi_address: '', pi_key: '', demo_model: 'pro' };

  function opt(label, small, ctl) {
    return h('div', { class: 'opt' }, h('div', { class: 'lab' }, label, small ? h('small', null, small) : null), ctl);
  }
  function cfgSwitch(path, label) {
    const get2 = () => path.split('.').reduce((o, k) => o[k], S.config);
    const b = sw(get2(), () => {
      const v = !get2();
      const patch = {};
      path.split('.').reduce((o, k, i, a) => (o[k] = i === a.length - 1 ? v : {}), patch);
      saveConfig(patch).then(() => b.setAttribute('aria-checked', get2() ? 'true' : 'false'));
    }, label);
    pageSubs.add(() => b.setAttribute('aria-checked', get2() ? 'true' : 'false'));
    return b;
  }
  function cfgSelect(path, choices, label) {
    const get2 = () => path.split('.').reduce((o, k) => o[k], S.config);
    const sel = h('select', { 'aria-label': label, onchange: () => {
      const patch = {};
      const v = typeof choices[0][0] === 'number' ? Number(sel.value) : sel.value;
      path.split('.').reduce((o, k, i, a) => (o[k] = i === a.length - 1 ? v : {}), patch);
      saveConfig(patch);
    } }, choices.map(([v, t]) => h('option', { value: v, text: t })));
    sel.value = String(get2());
    pageSubs.add(() => { sel.value = String(get2()); });
    return sel;
  }

  function pageSettings() {
    const cfg = S.config;
    // connection: the active saved unit
    const u = unit();
    const mode = u.mode;
    const unitSel = h('select', { 'aria-label': 'Saved connection', onchange: () => saveConfig({ active: Number(unitSel.value) }).then(drawPage) },
      cfg.units.map((x, i) => h('option', { value: i, text: unitLabel(x, i) })));
    unitSel.value = String(cfg.active);
    const unitName = h('input', { type: 'text', placeholder: 'Name (optional)', value: u.name || '', maxlength: 24, style: { width: '160px' }, 'aria-label': 'Connection name' });
    unitName.addEventListener('change', () => saveUnit({ name: unitName.value.trim() }).then(drawPage));
    const addUnit = h('button', { class: 'btn', title: 'Another saved connection; switch between them in the top bar or the tray', onclick: () => saveConfig({ units: [...cfg.units, { ...NEW_UNIT }], active: cfg.units.length }).then(drawPage) }, 'Add');
    const rmUnit = h('button', { class: 'btn', title: 'Remove this saved connection', disabled: cfg.units.length < 2, onclick: async () => {
      if (!(await confirmDlg('Remove this connection?', `${unitLabel(u, cfg.active)} is removed from the saved connections.`, 'Remove'))) return;
      saveConfig({ units: cfg.units.filter((_, i) => i !== cfg.active), active: 0 }).then(drawPage);
    } }, 'Remove');
    const unitsRow = h('div', { class: 'inline', style: { marginBottom: '10px' } }, h('span', { class: 'lab' }, 'Connection'), unitSel, unitName, addUnit, rmUnit);
    const addr = h('input', { type: 'text', placeholder: 'adi2-pi.local or 192.168.1.20', value: u.pi_address, style: { width: '240px' } });
    const keyIn = h('input', { type: 'password', placeholder: 'only if set on the ADI-2-Pi', value: u.pi_key || '', autocomplete: 'off', style: { width: '240px' }, 'aria-label': 'Access key' });
    const keyShow = h('button', { class: 'btn', title: 'Show or hide the key', onclick: () => { keyIn.type = keyIn.type === 'password' ? 'text' : 'password'; } }, '👁');
    const keyState = h('div', { class: 'hint', style: { margin: '4px 0 0' } });
    const drawKeyState = () => {
      const p = S.device.problem;
      const pi = unit().mode === 'pi';
      const bad = pi && p && (p.code === 'pi-auth' || p.code === 'pi-locked');
      keyState.className = `hint${bad ? ' bad' : ''}`;
      keyState.textContent = bad ? p.text
        : pi && S.device.connected && S.device.read_only ? 'View only: this ADI-2-Pi shows its state without a key; changes need the key set on its System tab.'
          : pi && S.device.connected && unit().pi_key ? 'Key accepted.' : 'Set on the ADI-2-Pi\'s System tab. Leave empty if the bridge has none.';
    };
    drawKeyState();
    pageSubs.add(drawKeyState);
    const connectPi = () => {
      if (!addr.value.trim()) { toast('Enter the ADI-2-Pi address, or press Find'); return; }
      saveUnit({ mode: 'pi', pi_address: addr.value.trim(), pi_key: keyIn.value.trim() });
    };
    keyIn.addEventListener('keydown', (e) => { if (e.key === 'Enter') connectPi(); });
    const found = h('div', { class: 'found' });
    const find = h('button', { class: 'btn', onclick: () => {
      find.disabled = true;
      found.replaceChildren(h('span', { class: 'hint' }, 'Looking for ADI-2-Pi on the network…'));
      invoke('discover').then((list) => {
        found.replaceChildren(...(list.length ? list.map((f) => h('button', { class: 'btn', onclick: () => { addr.value = f.address; connectPi(); } },
          `${f.name} – ${f.address}${f.ip ? ` (${f.ip})` : ''}`)) : [h('span', { class: 'hint' }, 'None found. Type the address instead.')]));
      }).catch((e) => found.replaceChildren(h('span', { class: 'hint' }, String(e)))).finally(() => { find.disabled = false; });
    } }, 'Find');
    const radio = (v, label, small, ...extra) => h('label', null,
      h('input', { type: 'radio', name: 'mode', value: v, checked: mode === v, onchange: () => {
        if (v === 'pi' && !addr.value.trim()) { toast('Enter the ADI-2-Pi address, or press Find'); return; }
        saveUnit({ mode: v, pi_address: addr.value.trim(), pi_key: keyIn.value.trim() });
      } }),
      h('div', null, h('div', null, label), small ? h('div', { class: 'hint', style: { margin: 0 } }, small) : null, extra));
    const demoModel = h('select', { 'aria-label': 'Demo model', onchange: () => saveUnit({ demo_model: demoModel.value }) },
      [['se', 'ADI-2/4 Pro SE'], ['pro', 'ADI-2 Pro'], ['dac', 'ADI-2 DAC']].map(([v, t]) => h('option', { value: v, text: t })));
    demoModel.value = u.demo_model || 'pro';
    const piLink = h('a', { href: '#', title: 'github.com/shells-dw/adi2-pi', onclick: (e) => { e.preventDefault(); invoke('open_adi2_pi_page').catch((err) => toast(err, true)); } }, 'ADI-2-Pi');
    const conn = h('section', { class: 'panel' }, h('h2', null, 'Connection'),
      unitsRow,
      h('div', { class: 'radio' },
        radio('usb', 'USB on this computer', 'Through the ALSA sequencer, next to PipeWire or JACK. Set MIDI Control to ON on the unit.'),
        radio('pi', 'ADI-2-Pi on the network', ['An ADI-2 connected to a Raspberry Pi running ', piLink, '. The bridge\'s own volume limits apply too.'],
          h('div', { class: 'inline', style: { marginTop: '6px' } }, addr, find,
            h('button', { class: 'btn', onclick: connectPi }, 'Connect')),
          h('div', { class: 'inline', style: { marginTop: '6px' } }, keyIn, keyShow, h('span', { class: 'lab' }, 'Access key')),
          keyState, found),
        radio('demo', 'Demo', 'A simulated unit, to look around without one.',
          h('div', { class: 'inline', style: { marginTop: '6px' } }, demoModel))));

    // volume
    const outs = outputs().filter((c) => !S.device.model || shows(key(c.addr, 12), SEC.output.vol, c));
    const numIn = (val, onch, attrs) => { const i = h('input', { type: 'number', step: 0.5, value: val ?? '', placeholder: 'none', ...attrs }); i.addEventListener('change', () => onch(i.value === '' ? null : Number(i.value))); return i; };
    const profiles = cfg.volume.profiles;
    const pIdx = cfg.volume.profile;
    const prof = profiles[pIdx] || profiles[0];
    const profSel = h('select', { 'aria-label': 'Volume limit profile', onchange: () => saveProfiles(profiles, Number(profSel.value)).then(drawPage) },
      profiles.map((p, i) => h('option', { value: i, text: p.name })));
    profSel.value = String(pIdx);
    const profName = h('input', { type: 'text', value: prof.name, maxlength: 24, style: { width: '140px' }, 'aria-label': 'Profile name' });
    profName.addEventListener('change', () => saveProfiles(profiles.map((p, i) => (i === pIdx ? { ...p, name: profName.value.trim() || p.name } : p))).then(drawPage));
    const addProf = h('button', { class: 'btn', title: 'Another set of limits; switch between them in the tray menu', onclick: () => saveProfiles([...profiles, { name: `Profile ${profiles.length + 1}`, limit: { ...prof.limit } }], profiles.length).then(drawPage) }, 'Add');
    const rmProf = h('button', { class: 'btn', disabled: profiles.length < 2, onclick: () => saveProfiles(profiles.filter((_, i) => i !== pIdx), 0).then(drawPage) }, 'Remove');
    const setLimit = (ch, v) => saveProfiles(profiles.map((p, i) => (i === pIdx ? { ...p, limit: { ...p.limit, [ch]: v } } : p)));
    const volRows = outs.map((c) => h('tr', null,
      h('td', null, chName(c)),
      h('td', null, numIn(prof.limit[c.key], (v) => setLimit(c.key, v), { min: -114.5, max: 6, 'aria-label': `${chName(c)} limit` }), ' dB'),
      h('td', null, numIn(cfg.volume.recall[c.key], (v) => saveConfig({ volume: { recall: { [c.key]: v } } }), { min: -114.5, max: 6, 'aria-label': `${chName(c)} recall` }), ' dB',
        h('button', { class: 'btn', style: { marginLeft: '6px' }, onclick: () => { const v = get(key(c.addr, 12)); if (v !== undefined) saveConfig({ volume: { recall: { [c.key]: v / 10 } } }).then(() => drawPage()); } }, 'Use current'))));
    const vol = h('section', { class: 'panel' }, h('h2', null, 'Volume'),
      h('div', { class: 'inline', style: { marginBottom: '8px' } }, h('span', { class: 'lab' }, 'Limit profile'), profSel, profName, addProf, rmProf),
      h('table', { class: 'tbl' }, h('tr', null, h('td', null, 'Output'), h('td', null, 'Limit'), h('td', null, 'Recall')), volRows),
      h('div', { class: 'opts', style: { marginTop: '10px' } },
        opt('Limit volume rise speed', 'Raising the volume in this app, with the keys or the tray', cfgSelect('volume.rise', [[0, 'Off'], [20, '20 dB/s'], [10, '10 dB/s'], [5, '5 dB/s']], 'Rise speed')),
        opt('Step', 'Volume keys, tray and the +/− buttons', cfgSelect('volume.step', [[0.5, '0.5 dB'], [1, '1 dB'], [2, '2 dB'], [3, '3 dB']], 'Step'))),
      h('p', { class: 'hint' }, 'Limit, link and rise speed apply to changes made in this app, not to the knob on the unit. With more than one profile the tray menu switches between them.'));

    // keys
    const keysBox = h('div', { class: 'hint' }, 'Checking…');
    invoke('keys_status').then((st) => {
      const cmds = h('div', { class: 'cmds' }, st.commands.map(([a, cmd]) => [h('span', null, { 'volume-up': 'Volume up', 'volume-down': 'Volume down', mute: 'Mute' }[a]), h('code', null, cmd)]));
      if (st.method === 'gnome') {
        keysBox.replaceChildren(h('span', null, st.active ? 'The volume keys are bound to this app (GNOME keyboard settings). Switching this off gives them back to GNOME.' : 'On GNOME the app binds the keys itself.'),
          h('details', null, h('summary', null, 'Commands'), cmds));
      } else {
        keysBox.replaceChildren(h('span', null, `${st.desktop || 'This desktop'}: bind the volume keys to these commands in the keyboard shortcut settings (on KDE Plasma first clear the keys under Shortcuts → Audio Volume, then add them as commands):`), cmds);
      }
    }).catch(() => {});
    const keys = h('section', { class: 'panel' }, h('h2', null, 'Volume keys'),
      h('div', { class: 'opts' },
        opt('Volume keys control the ADI-2', null, cfgSwitch('keys.enabled', 'Volume keys')),
        opt('When', null, cfgSelect('keys.when', [['output', 'While the ADI-2 is the sound output'], ['always', 'Always']], 'When')),
        opt('Output', 'Also in the tray menu', cfgSelect('keys.target', [['auto', 'The output turned last'], ...outs.map((c) => [c.key, chName(c)])], 'Output'))),
      h('p', { class: 'hint' }, 'Otherwise the keys change the system volume as usual. Turn this off before removing the app.'), keysBox);

    // popup
    const style = h('span', { class: 'hint', style: { margin: 0 } });
    invoke('osd_style').then((st) => { style.textContent = { kde: 'Plasma\'s own popup', window: 'a small window', notification: 'a notification' }[st] || st; });
    const popup = h('section', { class: 'panel' }, h('h2', null, 'Volume popup'),
      h('div', { class: 'opts' },
        opt('Show the volume when it changes', null, cfgSwitch('osd.enabled', 'Popup')),
        opt('Also when turned on the unit', null, cfgSwitch('osd.knob', 'Knob')),
        opt('Style', h('span', null, 'Automatic here: ', style), cfgSelect('osd.style', [['auto', 'Automatic'], ['window', 'Window (X11)'], ['notification', 'Notification'], ['kde', 'KDE Plasma']], 'Style'))),
      h('div', { class: 'inline', style: { marginTop: '8px' } }, h('button', { class: 'btn', onclick: () => invoke('osd_test') }, 'Show it')));

    // app
    const app = h('section', { class: 'panel' }, h('h2', null, 'App'),
      h('div', { class: 'opts' },
        opt('Tray icon', 'Volume, mute, EQ and setups from the panel', cfgSwitch('tray', 'Tray')),
        opt('Closing the window keeps the app in the tray', null, cfgSwitch('close_to_tray', 'Close to tray')),
        opt('Start at login', null, cfgSwitch('autostart', 'Autostart')),
        opt('Start without the window', 'Tray only', cfgSwitch('start_hidden', 'Start hidden')),
        opt('Mouse wheel changes', null, cfgSelect('ui.wheel', [['all', 'All knobs'], ['volume', 'Volume only'], ['off', 'Nothing']], 'Mouse wheel')),
        opt('Follow the unit', 'Switch to the output whose volume is turned on the unit', cfgSwitch('ui.follow', 'Follow the active output')),
        opt('Theme', null, cfgSelect('ui.theme', [['dark', 'Dark'], ['light', 'Light'], ['system', 'Same as the desktop']], 'Theme')),
        opt('Compact layout', 'Smaller panels and knobs', cfgSwitch('ui.compact', 'Compact')),
        h('div', { class: 'opt' }, h('div', { class: 'lab' }, 'Keyboard shortcuts', h('small', null, 'In the window')), h('button', { class: 'btn', onclick: showShortcuts }, 'Show')),
        updatePanelPart()),
      h('p', { class: 'hint' }, 'On GNOME the tray icon needs the AppIndicator extension (Ubuntu has it).'),
      h('div', { class: 'about' },
        h('div', null, h('b', null, `Open ADI-2 Control ${S.version}`), ' · GPL-3.0-or-later · not affiliated with RME'),
        h('div', null, 'Updates, help and bug reports: ',
          h('a', { href: '#', onclick: (e) => { e.preventDefault(); invoke('open_project_page').catch((err) => toast(err, true)); } }, 'github.com/shells-dw/open-adi2-control'))));

    const statusTable = h('table', { class: 'tbl' });
    const drawStatus = () => {
      const st = S.status;
      const d = S.device;
      const row = (k, v) => h('tr', null, h('td', null, k), h('td', null, v));
      const yes = (b) => (b ? 'yes' : 'no');
      const pro = d.model === 'pro' || d.model === 'se';
      const modeNames = (SEC.device.basic && namesOf(SEC.device.basic)) || [];
      const outName = { 0: 'Line Out', 1: chShort(CH.ph12), 2: chShort(CH.ph34) };
      statusTable.replaceChildren(
        row('Model', d.model ? S.schema.modelNames[d.model] : '–'),
        row('Device ID', d.id ? `0x${d.id.toString(16)}` : '–'),
        row('Connected through', d.port || '–'),
        unit().mode === 'pi' ? row('Access', d.connected ? (d.read_only ? 'view only (no key)' : 'full') : '–') : null,
        row('Device mode', st ? `${modeNames[st.mode] ?? '?'} (${st.mode})` : '–'),
        row('Balanced phones', st ? yes(st.balanced) : '–'),
        row('DSD decode', st ? String(st.dsd) : '–'),
        row('Active output', st ? `${st.active_out}${outName[st.active_out] ? ` · ${outName[st.active_out]}?` : ''}` : '–'),
        row('Active line', st ? String(st.active_line) : '–'),
        pro ? row('AES input signal', st ? yes(st.aes_in) : '–') : null,
        pro ? row('SPDIF input signal', st ? yes(st.spdif_in) : '–') : null,
        row('Revision / protocol', st ? `${st.revision} / ${st.protocol}` : '–'),
        row('IOStatus 1 / 2', st ? h('code', null, `${st.io1.toString(16).padStart(7, '0')} ${st.io2.toString(16).padStart(7, '0')}`) : '–'),
        row('Status message', h('code', null, st ? st.hex : '–')));
    };
    drawStatus();
    pageSubs.add(drawStatus);
    const status = h('section', { class: 'panel' }, h('h2', null, 'Unit'),
      statusTable,
      h('p', { class: 'hint' }, 'Fields after "Device mode" are read from the unit\'s status message; "Active output" and the IOStatus words are only partly documented.'),
      h('div', { class: 'inline', style: { marginTop: '8px' } }, h('button', { class: 'btn', onclick: () => invoke('refresh').then(() => toast('Reading all settings again')).catch((e) => toast(e, true)) }, 'Read everything again')));

    return h('div', { class: 'page' }, h('div', { class: 'row2' }, conn, keys), h('div', { class: 'row2' }, vol, popup), h('div', { class: 'row2' }, app, status));
  }

  // ---------------------------------------------------------------- updates
  /** Release notes (GitHub Markdown) as plain elements: paragraphs, "- " lists, **bold**, `code`, headings. */
  function mdLite(text) {
    const inline = (line) => {
      const out = [];
      const re = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
      let last = 0;
      for (const m of line.matchAll(re)) {
        if (m.index > last) out.push(line.slice(last, m.index));
        const t = m[0];
        if (t.startsWith('**')) out.push(h('b', null, t.slice(2, -2)));
        else if (t.startsWith('`')) out.push(h('code', null, t.slice(1, -1)));
        else out.push(t.slice(1, t.indexOf(']'))); // link text only
        last = m.index + t.length;
      }
      if (last < line.length) out.push(line.slice(last));
      return out;
    };
    const box = h('div', { class: 'notes' });
    let list = null;
    for (const raw of String(text || '').replace(/\r/g, '').split('\n')) {
      const line = raw.trimEnd();
      const li = line.match(/^\s*[-*] (.*)$/);
      if (li) {
        if (!list) { list = h('ul'); box.append(list); }
        list.append(h('li', null, inline(li[1])));
        continue;
      }
      list = null;
      if (!line.trim()) continue;
      const hd = line.match(/^#{1,6} (.*)$/);
      box.append(hd ? h('h4', null, inline(hd[1])) : h('p', null, inline(line)));
    }
    if (!box.childElementCount) box.append(h('p', { class: 'hint' }, 'No release notes.'));
    return box;
  }

  const openRelease = (url) => invoke('open_release_link', { url }).catch((e) => toast(e, true));

  function installHint(u, r) {
    if (u.install === 'deb' && r.download) return h('p', { class: 'hint' }, 'Download it, then install it over this version: ', h('code', null, `sudo apt install ./${r.download.name}`), '. Settings are kept.');
    if (u.install === 'appimage' && r.download) return h('p', { class: 'hint' }, 'Download it and replace ', h('code', null, u.appimage || 'your AppImage'), ' with it (make it executable). Settings are kept.');
    if (u.install === 'rpm' && r.download) return h('p', { class: 'hint' }, 'Download it, then install it over this version: ', h('code', null, `sudo dnf install ./${r.download.name}`), ' (or zypper). Settings are kept.');
    if (u.install === 'pacman') return h('p', { class: 'hint' }, 'Installed from the AUR: update it with your AUR helper once the package is updated.');
    if (u.install === 'flatpak') return h('p', { class: 'hint' }, 'Installed as a Flatpak: ', h('code', null, 'flatpak update'), ' once the Flatpak is updated.');
    return h('p', { class: 'hint' }, 'Get it from the release page.');
  }

  function showRelease(r) {
    const u = S.update || {};
    const d = dialog(`${r.name}`, h('div', { style: { maxWidth: '560px' } },
      h('div', { class: 'hint', style: { marginTop: 0 } }, `Version ${r.version}${r.published ? ` · ${r.published}` : ''} · you have ${S.version}`),
      h('div', { class: 'notes-box' }, mdLite(r.notes)),
      installHint(u, r)), [
      h('button', { class: 'btn', onclick: () => { d.close(); invoke('update_skip', { version: r.version }).then((v) => { S.update = v; renderUpdBar(); }); } }, 'Skip this version'),
      h('button', { class: 'btn', onclick: () => openRelease(r.url) }, 'Release page'),
      r.download ? h('button', { class: 'btn primary', onclick: () => openRelease(r.download.url) }, `Download ${r.download.name.endsWith('.deb') ? '.deb' : r.download.name.endsWith('.rpm') ? '.rpm' : 'AppImage'}`) : null,
      h('button', { class: 'btn', autofocus: true, onclick: () => d.close() }, 'Close'),
    ]);
  }

  function renderUpdBar() {
    const bar = $('#updBar');
    const r = S.update && S.update.offer;
    if (!r || S.updDismissed === r.version) { bar.hidden = true; return; }
    bar.hidden = false;
    bar.replaceChildren(
      h('span', null, h('b', null, `Version ${r.version}`), ' of Open ADI-2 Control is available.'),
      h('button', { class: 'btn', onclick: () => showRelease(r) }, 'What\'s new'),
      h('button', { class: 'linkbtn', title: 'Hide until the next start', onclick: () => { S.updDismissed = r.version; renderUpdBar(); } }, '✕'));
  }

  /** Settings → App: switch, last result, Check now. */
  function updatePanelPart() {
    const line = h('div', { class: 'hint', style: { margin: 0 } });
    const btn = h('button', { class: 'btn', onclick: () => {
      btn.disabled = true;
      invoke('update_check').then((v) => { S.update = v; renderUpdBar(); draw(); }).catch((e) => toast(e, true)).finally(() => { btn.disabled = false; });
    } }, 'Check now');
    const extra = h('div', { class: 'inline' });
    function draw() {
      const u = S.update || {};
      const when = u.checkedAt ? new Date(u.checkedAt * 1000) : null;
      const ago = when ? (when.toDateString() === new Date().toDateString() ? `today ${when.toTimeString().slice(0, 5)}` : when.toLocaleDateString()) : null;
      extra.replaceChildren();
      line.className = 'hint';
      if (u.checking) line.textContent = 'Asking GitHub…';
      else if (u.error) { line.textContent = `Last check${ago ? ` (${ago})` : ''}: ${u.error}`; line.className = 'hint bad'; }
      else if (u.latest && u.newer) {
        line.textContent = `Version ${u.latest.version} is available (checked ${ago}).`;
        extra.append(h('button', { class: 'btn primary', onclick: () => showRelease(u.latest) }, 'What\'s new'));
      } else if (u.latest) line.textContent = `This is the latest version (checked ${ago}).`;
      else line.textContent = 'Not checked yet.';
      btn.disabled = !!u.checking;
    }
    draw();
    pageSubs.add(draw);
    return [
      opt('Check GitHub for new versions', 'Once a day. Only asks GitHub for the latest version number; nothing is downloaded or installed.', cfgSwitch('updates.check', 'Check for updates')),
      h('div', { class: 'opt' }, line, h('div', { class: 'inline', style: { flexWrap: 'nowrap' } }, extra, btn)),
    ];
  }

  // ---------------------------------------------------------------- theme, shortcuts, follow
  const unitLabel = (x, i) => x.name || ({ usb: 'USB', pi: x.pi_address || 'ADI-2-Pi', demo: 'Demo' }[x.mode] || `Connection ${i + 1}`);

  const systemLight = window.matchMedia('(prefers-color-scheme: light)');
  function applyTheme() {
    const ui = (S.config && S.config.ui) || {};
    const theme = ui.theme === 'system' ? (systemLight.matches ? 'light' : 'dark') : ui.theme === 'light' ? 'light' : 'dark';
    const root = document.documentElement;
    if (root.dataset.theme !== theme) {
      root.dataset.theme = theme;
      for (const k of Object.keys(CSSC)) delete CSSC[k];
      document.querySelectorAll('canvas.curve').forEach((cv) => cv.dispatchEvent(new Event('themechange')));
    }
    root.toggleAttribute('data-compact', !!ui.compact);
  }
  systemLight.addEventListener('change', applyTheme);

  /** The output the shortcuts act on: the page's output, else the one the volume keys control. */
  function shortcutOutput() {
    if (CH[S.page] && CH[S.page].kind === 'output') return CH[S.page];
    const t = S.config.keys.target;
    const c = t === 'auto' ? CH[S.lastOut] : CH[t];
    return c && has(key(c.addr, 12)) ? c : outputs().find((o) => has(key(o.addr, 12)));
  }

  const SHORTCUTS = [
    ['↑ / ↓', 'Volume up / down by the step'],
    ['Page ↑ / Page ↓', 'Volume up / down by 5 dB'],
    ['M', 'Mute'], ['D', 'Dim'], ['E', 'EQ on / off'],
    ['1 – 9', 'Load setup 1 – 9 (asks first)'],
    ['Ctrl + Page ↑ / ↓', 'Previous / next tab'],
    ['?', 'This list'],
  ];
  function showShortcuts() {
    const d = dialog('Keyboard shortcuts', h('table', { class: 'tbl' }, SHORTCUTS.map(([k, t]) => h('tr', null, h('td', null, h('code', null, k)), h('td', null, t)))),
      [h('button', { class: 'btn', onclick: () => d.close() }, 'Close')]);
    d.querySelector('p') && d.querySelector('p').remove();
  }

  function onKey(e) {
    if (e.defaultPrevented || $('#dlg').open || e.target.matches('input, select, textarea')) return;
    if (e.ctrlKey && (e.key === 'PageUp' || e.key === 'PageDown')) {
      const tabs = tabList();
      const i = tabs.findIndex((t) => t.id === S.page);
      const next = tabs[(i + (e.key === 'PageDown' ? 1 : tabs.length - 1)) % tabs.length];
      if (next) go(next.id);
      e.preventDefault();
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === '?') { showShortcuts(); e.preventDefault(); return; }
    if (!online()) return;
    const c = shortcutOutput();
    const vol = (dir, db) => {
      if (!c) return;
      const k = key(c.addr, 12);
      const v = get(k);
      if (v !== undefined) setRaw(k, v + dir * Math.round(db * 10));
    };
    const tog = (k) => { if (c && has(k)) setRaw(k, get(k) ? 0 : 1); };
    switch (e.key) {
      case 'ArrowUp': vol(1, S.config.volume.step || 1); break;
      case 'ArrowDown': vol(-1, S.config.volume.step || 1); break;
      case 'PageUp': vol(1, 5); break;
      case 'PageDown': vol(-1, 5); break;
      case 'm': case 'M': tog(key(c.addr, 15)); break;
      case 'd': case 'D': tog(key(c.addr, 16)); break;
      case 'e': case 'E': tog(key(c.eq, 2)); break;
      default:
        if (/^[1-9]$/.test(e.key) && S.setups.length) { confirmSetup(Number(e.key)); break; }
        return;
    }
    e.preventDefault();
  }

  /** Follow the output the unit is working on (knob turned, or the status says so). */
  function followOutput(chKey) {
    if (!S.config || S.config.ui.follow === false || !chKey || !CH[chKey] || S.page === chKey) return;
    if (!(CH[S.page] && CH[S.page].kind === 'output')) return; // only while an output page is open
    if ($('#dlg').open) return;
    go(chKey);
  }

  // ---------------------------------------------------------------- tabs / render
  function tabList() {
    const t = [];
    if (avail(CH.in)) t.push({ id: 'in', label: chShort(CH.in) });
    for (const c of outputs()) if (avail(c)) t.push({ id: c.key, label: chShort(c) });
    t.push({ id: 'device', label: 'Device', sep: true }, { id: 'presets', label: 'EQ Presets' });
    if (window.HP) t.push({ id: 'hp', label: 'Headphone EQ' });
    t.push({ id: 'settings', label: 'Settings', sep: true });
    return t;
  }
  function renderTabs() {
    const tabs = tabList();
    if (!tabs.some((t) => t.id === S.page)) S.page = 'line';
    $('#tabs').replaceChildren(...tabs.map((t) => h('button', { class: `tab${t.sep ? ' sep' : ''}`, role: 'tab', 'aria-selected': t.id === S.page ? 'true' : 'false', onclick: () => go(t.id) }, t.label)));
  }
  function go(page) {
    if (S.page === page) return;
    S.page = page;
    saveConfig({ ui: { page } });
    renderTabs();
    drawPage();
    $('#main').scrollTop = 0;
  }

  function drawPage() {
    subs.clear();
    pageSubs.clear();
    const main = $('#main');
    let page;
    if (!S.schema || !S.config) return;
    if (DEBUG) log(`draw ${S.page} online=${online()} usable=${usable()} p=${S.p.size} ready=${S.device.ready}`);
    const unitPage = !['settings', 'presets', 'hp'].includes(S.page);
    if (unitPage && !S.device.connected) {
      page = h('div', { class: 'empty-page' },
        h('div', { class: 'big' }, unit().mode === 'pi' ? 'ADI-2-Pi not reachable' : 'No ADI-2 connected'),
        h('p', null, (S.device.problem && S.device.problem.text) || 'Looking for the unit…'),
        h('div', { class: 'inline', style: { justifyContent: 'center', marginTop: '14px' } },
          h('button', { class: 'btn', onclick: () => go('settings') }, 'Connection settings'),
          unit().mode !== 'demo' ? h('button', { class: 'btn', onclick: () => saveUnit({ mode: 'demo' }) }, 'Try the demo') : null));
    } else if (S.page === 'in') page = pageInput();
    else if (CH[S.page] && CH[S.page].kind === 'output') page = pageOutput(CH[S.page]);
    else if (S.page === 'device') page = pageDevice();
    else if (S.page === 'presets') page = pagePresets();
    else if (S.page === 'hp') page = window.HP ? window.HP.page(ctx()) : h('div', { class: 'empty-page' }, 'Headphone EQ is not available in this build.');
    else page = pageSettings();
    main.replaceChildren(page);
    main.classList.toggle('locked', !online());
  }

  /** What the headphone EQ tab (hp.js) needs from here. */
  const ctx = () => ({ h, s, S, invoke, toast, dialog, confirmDlg, field, opt, knob, fmtHz, importPresets, model, chName, outputs, go, cssColor, usable, guard, get, has, key, SEC, CH });

  function renderConn() {
    const el = $('#conn');
    const d = S.device;
    let text, cls;
    if (!d.connected) { text = (d.problem && d.problem.text) || 'Not connected'; cls = 'bad'; }
    else if (!d.answering) { text = (d.problem && d.problem.text) || 'Waiting for the unit…'; cls = 'wait'; }
    else if (!d.ready) { text = 'Reading settings…'; cls = 'wait'; }
    else {
      const sr = SEC.device.srate && get(key(12, SEC.device.srate.idx));
      const srn = sr !== undefined && namesOf(SEC.device.srate) ? namesOf(SEC.device.srate)[sr] : null;
      const via = unit().mode === 'pi' ? ' · via ADI-2-Pi' : unit().mode === 'demo' ? ' · Demo' : '';
      text = `${S.schema.modelNames[d.model] || 'ADI-2'}${srn ? ` · ${srn}` : ''}${via}`;
      cls = 'ok';
    }
    el.className = `conn ${cls}`;
    $('#connText').textContent = text;
    el.title = d.port || '';
    // view-only badge (ADI-2-Pi without its key)
    const badge = $('#viewOnly');
    badge.hidden = !(d.connected && d.read_only);
    // saved connections
    const us = $('#units');
    const units = (S.config && S.config.units) || [];
    us.hidden = units.length < 2;
    if (units.length > 1) {
      us.replaceChildren(...units.map((x, i) => h('option', { value: i, text: unitLabel(x, i) })));
      us.value = String(S.config.active);
    }
    const banner = $('#banner');
    const show = d.connected && !d.answering && d.problem;
    banner.hidden = !show;
    if (show) banner.textContent = d.problem.text;
    $('#main').classList.toggle('locked', !online());
  }

  // ---------------------------------------------------------------- updates from the app
  let lastShape = '';
  let redrawTimer = 0;
  function scheduleRedraw() {
    clearTimeout(redrawTimer);
    redrawTimer = setTimeout(() => { if (!$('#dlg').open) drawPage(); else scheduleRedraw(); }, 200);
  }
  const shape = () => JSON.stringify([S.device.connected, S.device.ready, S.device.model, S.device.answering && S.device.connected, S.device.read_only]);

  function onUpdate(u) {
    if (DEBUG) log(`update ${u.kind}${u.kind === 'device' ? ` ${JSON.stringify(u.device)}` : u.kind === 'params' ? ` ${Object.keys(u.p).length} keys` : ''}`);
    switch (u.kind) {
      case 'reset':
        S.p.clear();
        break;
      case 'device': {
        S.device = u.device;
        renderConn();
        if (S.page === 'settings') for (const fn of pageSubs) fn();
        const sh = shape();
        if (sh !== lastShape) { lastShape = sh; renderTabs(); drawPage(); }
        break;
      }
      case 'params': {
        const ks = Object.keys(u.p);
        // a parameter the page didn't know when it was drawn (the unit reported it after
        // "ready"): draw the page again so its control appears
        const fresh = S.device.ready && ks.some((k) => !S.p.has(k));
        for (const k of ks) S.p.set(k, u.p[k]);
        if (fresh) scheduleRedraw();
        if (u.origin === 'device' && S.device.ready) {
          for (const c of outputs()) if (u.p[key(c.addr, 12)] !== undefined) S.lastOut = c.key;
        }
        if (u.turned) followOutput(u.turned);
        notify(ks);
        if (SEC.device.srate && ks.includes(key(12, SEC.device.srate.idx))) renderConn();
        break;
      }
      case 'setups':
        S.setups = u.setups;
        for (const fn of pageSubs) fn();
        break;
      case 'presets':
        S.presets = u.presets;
        if (S.editor && !editorChanged() && S.page === 'presets') {
          const n = S.editor.n;
          presetToEditor(n);
          drawPage();
        } else for (const fn of pageSubs) fn();
        break;
      case 'status': {
        const prev = S.status;
        S.status = u.status;
        // ADI-2 DAC: byte 6 bits 1–0 name the active output (0 Line, 1 Phones, 2 IEM assumed)
        if (prev && u.status && prev.active_out !== u.status.active_out && S.device.model === 'dac') followOutput(['line', 'ph12', 'ph34'][u.status.active_out]);
        if (S.page === 'settings') for (const fn of pageSubs) fn();
        break;
      }
      case 'updates':
        S.update = u.update;
        renderUpdBar();
        if (S.page === 'settings') for (const fn of pageSubs) fn();
        break;
      case 'config': {
        const before = S.config && JSON.stringify([unit(), S.config.units.length]);
        S.config = u.config;
        applyTheme();
        const unitChanged = before !== JSON.stringify([unit(), S.config.units.length]);
        if (unitChanged) { S.editor = null; S.ab = null; renderConn(); drawPage(); } else for (const fn of pageSubs) fn();
        break;
      }
      default:
        break;
    }
  }

  const log = (msg) => invoke('ui_log', { msg: String(msg) }).catch(() => {});
  if (window.HP && DEBUG) window.HP.log = log;
  window.addEventListener('error', (e) => { log(`error: ${e.message} (${e.filename}:${e.lineno})`); toast(`Window error: ${e.message}`, true); });
  window.addEventListener('unhandledrejection', (e) => { log(`rejection: ${e.reason && e.reason.stack || e.reason}`); toast(`Window error: ${e.reason}`, true); });

  // ---------------------------------------------------------------- start
  async function start() {
    const [schemaText, snap] = await Promise.all([invoke('schema_json'), invoke('snapshot')]);
    S.schema = JSON.parse(schemaText);
    for (const sec of ['output', 'input', 'eq', 'device']) SEC[sec] = Object.fromEntries(S.schema.params[sec].map((p) => [p.id, p]));
    SEC.eqByIdx = Object.fromEntries(S.schema.params.eq.map((p) => [p.idx, p]));
    CH = Object.fromEntries(S.schema.channels.map((c) => [c.key, c]));
    if (DEBUG) log(`snapshot 1: p=${Object.keys(snap.p).length} device=${JSON.stringify(snap.device)}`);
    S.device = snap.device;
    S.p = new Map(Object.entries(snap.p));
    S.setups = snap.setups;
    S.presets = snap.presets;
    S.status = snap.status;
    S.config = snap.config;
    S.lastOut = snap.lastOut;
    S.version = snap.version;
    S.update = snap.update;
    applyTheme();
    renderUpdBar();
    S.page = S.config.ui.page || 'line';
    lastShape = shape();
    renderTabs();
    renderConn();
    drawPage();
    window.addEventListener('keydown', onKey);
    $('#viewOnly').addEventListener('click', askKey);
    $('#units').addEventListener('change', (e) => saveConfig({ active: Number(e.target.value) }));
    await listen('update', (e) => onUpdate(e.payload));
    await listen('ask', (e) => { if (e.payload && e.payload.setup) confirmSetup(e.payload.setup); });
    // anything that arrived between the snapshot and listening
    const again = await invoke('snapshot');
    if (DEBUG) log(`snapshot 2: p=${Object.keys(again.p).length}`);
    S.device = again.device;
    S.p = new Map(Object.entries(again.p));
    S.setups = again.setups;
    S.presets = again.presets;
    lastShape = shape();
    renderTabs();
    renderConn();
    drawPage();
  }
  start().catch((e) => { document.body.append(h('pre', { style: { color: '#f88', padding: '20px' } }, String(e && e.stack || e))); });
})();
