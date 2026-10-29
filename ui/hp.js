'use strict';
/*
 * Headphone EQ tab: an EQ for the ADI-2's five bands (and optionally Bass/Treble) from a headphone
 * measurement and a target curve. Measurements and targets: AutoEq snapshot shipped with the app
 * (eq_index / eq_file commands). Fitting: hp/core.js from ADI-2-Pi, in a worker (hp/worker.js)
 * or, where module workers are unavailable, on the main thread.
 *
 * The result goes to the EQ preset editor, straight to an output's EQ, into a preset slot, or
 * into a .adieqpr file.
 */
(() => {
  let C = null; // core.js module
  let worker = null;
  let workerOk = true;
  let jobId = 0;
  const csvCache = new Map();

  const OPT_KEYS = ['bassDb', 'bassHz', 'tiltDbOct', 'limitOn', 'limitHz', 'maxBands', 'maxBoost', 'maxCut', 'useBT', 'band1Type', 'band5Type', 'slowComp', 'slowLossDb', 'slowBand'];
  const STORE = 'hpeq';

  function load() {
    try { return JSON.parse(localStorage.getItem(STORE) || '{}'); } catch { return {}; }
  }
  function save(st) {
    try {
      localStorage.setItem(STORE, JSON.stringify({ hp: st.hp && st.hp.kind === 'repo' ? st.hp.path : null, target: st.targetAuto ? null : st.target && st.target.kind === 'repo' ? st.target.path : null, opt: Object.fromEntries(OPT_KEYS.map((k) => [k, st.opt[k]])), form: st.form }));
    } catch { /* storage unavailable */ }
  }

  async function core() {
    if (!C) C = await import('./hp/core.js');
    return C;
  }

  function runFit(meas, target, opt) {
    const id = ++jobId;
    if (workerOk && !worker) {
      try { worker = new Worker('hp/worker.js', { type: 'module' }); } catch { workerOk = false; }
    }
    if (workerOk && worker) {
      return new Promise((resolve, reject) => {
        const onMsg = (e) => {
          if (e.data.id !== id) return;
          worker.removeEventListener('message', onMsg);
          worker.removeEventListener('error', onErr);
          if (e.data.error) reject(new Error(e.data.error)); else resolve(e.data);
        };
        const onErr = (e) => {
          worker.removeEventListener('message', onMsg);
          worker.removeEventListener('error', onErr);
          e.preventDefault && e.preventDefault();
          // module workers not supported here: fit on the main thread from now on
          workerOk = false;
          worker = null;
          if (window.HP.log) window.HP.log('module worker unavailable, fitting on the main thread');
          resolve(runFit(meas, target, opt));
        };
        worker.addEventListener('message', onMsg);
        worker.addEventListener('error', onErr);
        worker.postMessage({ id, meas, target, opt });
      });
    }
    return core().then((K) => {
      const an = K.analyse(meas, target, opt);
      const fit = K.fit(an, opt);
      return { id, an, fit };
    });
  }

  /** ADI-2 EQ values (protocol integers by EQ index) of fitted filters. */
  function toPresetValues(filters, defaults) {
    const get = (id) => filters.find((f) => f.id === id && !f.off);
    const L = {};
    for (let i = 3; i <= 26; i++) if (i !== 20) L[i] = defaults[i];
    const idx = { B1: [4, 5, 6], B2: [7, 8, 9], B3: [10, 11, 12], B4: [13, 14, 15], B5: [17, 18, 19], BB: [21, 22, 23], BT: [24, 25, 26] };
    for (const [id, [g, f, q]] of Object.entries(idx)) {
      const fl = get(id);
      L[g] = fl ? Math.round(fl.gain * 2) : 0;
      if (fl) { L[f] = Math.round(fl.f); L[q] = Math.round(fl.q * 10); }
    }
    const b1 = get('B1'), b5 = get('B5');
    L[3] = b1 && b1.type === 'lowshelf' ? 1 : 0;
    L[16] = b5 && b5.type === 'highshelf' ? 1 : 0;
    return { L, bt: !!(get('BB') || get('BT')) };
  }

  function page(X) {
    const { h, S, invoke, toast } = X;
    const st = (S.hp ||= (() => {
      const saved = load();
      return { index: null, error: null, hp: null, target: null, targetAuto: !saved.target, savedHp: saved.hp, savedTarget: saved.target, form: saved.form || 'all', search: '', opt: null, savedOpt: saved.opt || {}, result: null, busy: false, name: '' };
    })());

    const root = h('div', { class: 'page hp-page' });
    const listBox = h('div', { class: 'hp-list', role: 'listbox', 'aria-label': 'Headphones' });
    const count = h('div', { class: 'hint' });
    const search = h('input', { type: 'search', placeholder: 'Search, e.g. HD 650', value: st.search, 'aria-label': 'Search headphones', oninput: () => { st.search = search.value; renderList(); } });
    const formSeg = h('div', { class: 'seg' }, [['all', 'All'], ['over-ear', 'Over-ear'], ['in-ear', 'In-ear']].map(([v, t]) =>
      h('button', { value: v, 'aria-pressed': st.form === v ? 'true' : 'false', onclick: () => { st.form = v; formSeg.querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', b.value === v ? 'true' : 'false')); renderList(); save(st); } }, t)));
    const ownFile = h('button', { class: 'btn', title: 'A frequency response measured yourself or from another source: CSV or text, frequency and level columns', onclick: async () => {
      try {
        const picked = await invoke('pick_file', { title: 'Headphone measurement', ext: ['csv', 'txt'], label: 'Frequency response' });
        if (!picked) return;
        const K = await core();
        const data = K.parseResponse(picked[1]);
        st.hp = { kind: 'file', name: picked[0].replace(/\.(csv|txt)$/i, ''), form: 'over-ear', data };
        if (st.targetAuto) autoTarget();
        renderList(); renderTarget(); run();
      } catch (e) { toast(e.message || e, true); }
    } }, 'Measurement file…');

    const targetSel = h('select', { 'aria-label': 'Target curve', onchange: () => {
      const t = st.index.targets.find((x) => x.path === targetSel.value);
      if (!t) return;
      st.target = { kind: 'repo', ...t };
      st.targetAuto = false;
      save(st); run();
    } });
    const targetAutoBtn = h('button', { class: 'btn', title: 'Harman 2018 for over-ear, Harman 2019 for in-ear', onclick: () => { st.targetAuto = true; autoTarget(); renderTarget(); save(st); run(); } }, 'Automatic');
    const targetFile = h('button', { class: 'btn', onclick: async () => {
      try {
        const picked = await invoke('pick_file', { title: 'Target curve', ext: ['csv', 'txt'], label: 'Frequency response' });
        if (!picked) return;
        const K = await core();
        st.target = { kind: 'file', name: picked[0].replace(/\.(csv|txt)$/i, ''), data: K.parseResponse(picked[1]) };
        st.targetAuto = false;
        renderTarget(); run();
      } catch (e) { toast(e.message || e, true); }
    } }, 'Target file…');
    const targetNote = h('div', { class: 'hint' });

    // options
    const num = (k, attrs) => {
      const i = h('input', { type: 'number', ...attrs, 'aria-label': k });
      i.value = st.opt ? st.opt[k] : '';
      i.addEventListener('change', () => { const v = Number(i.value); if (Number.isFinite(v)) { st.opt[k] = v; save(st); run(); } });
      i.sync = () => { i.value = st.opt[k]; };
      return i;
    };
    const sel = (k, choices) => {
      const s2 = h('select', { 'aria-label': k, onchange: () => { st.opt[k] = s2.value; save(st); run(); } }, choices.map(([v, t]) => h('option', { value: v, text: t })));
      s2.sync = () => { s2.value = st.opt[k]; };
      return s2;
    };
    const chk = (k) => {
      const b = h('button', { class: 'sw', role: 'switch', 'aria-label': k, onclick: () => { st.opt[k] = !st.opt[k]; b.sync(); save(st); run(); } });
      b.sync = () => b.setAttribute('aria-checked', st.opt[k] ? 'true' : 'false');
      return b;
    };
    const ctl = {
      bassDb: num('bassDb', { min: -6, max: 12, step: 0.5 }), bassHz: num('bassHz', { min: 40, max: 250, step: 5 }),
      tiltDbOct: num('tiltDbOct', { min: -2, max: 2, step: 0.1 }),
      limitOn: chk('limitOn'), limitHz: num('limitHz', { min: 500, max: 20000, step: 100 }),
      maxBands: num('maxBands', { min: 1, max: 7, step: 1 }),
      maxBoost: num('maxBoost', { min: 0, max: 12, step: 0.5 }), maxCut: num('maxCut', { min: 0, max: 12, step: 0.5 }),
      useBT: chk('useBT'),
      band1Type: sel('band1Type', [['peak', 'Peak'], ['lowshelf', 'Low shelf']]),
      band5Type: sel('band5Type', [['peak', 'Peak'], ['highshelf', 'High shelf']]),
      slowComp: chk('slowComp'), slowLossDb: num('slowLossDb', { min: 0.5, max: 12, step: 0.5 }),
      slowBand: sel('slowBand', [['B5', 'Band 5 as high shelf'], ['BT', 'Treble control']]),
    };
    const unit = (el, u) => h('span', { class: 'inline', style: { gap: '4px', flexWrap: 'nowrap' } }, el, h('span', { class: 'lab' }, u));
    const row = (label, small, ...c) => h('div', { class: 'opt hp-opt' }, h('div', { class: 'lab' }, label, small ? h('small', null, small) : null), h('div', { class: 'hp-ctl' }, c));
    const resetOpts = h('button', { class: 'btn', onclick: async () => { const K = await core(); st.opt = { ...K.DEFAULTS }; syncOpts(); save(st); run(); } }, 'Defaults');
    const options = h('details', { class: 'panel hp-opts' },
      h('summary', null, h('h2', null, 'Options')),
      h('div', { class: 'hp-opt-grid' },
        h('div', { class: 'opts' },
          row('Bass shelf', 'Added to the target', unit(ctl.bassDb, 'dB at'), unit(ctl.bassHz, 'Hz')),
          row('Tilt', 'Negative: warmer, positive: brighter', unit(ctl.tiltDbOct, 'dB/oct')),
          row('Correct only up to', 'Treble above depends on fit and ear shape', ctl.limitOn, unit(ctl.limitHz, 'Hz')),
          row('Compensate the Slow DA filter', 'About 3 dB at 20 kHz at 44.1/48 kHz', ctl.slowComp, unit(ctl.slowLossDb, 'dB'), ctl.slowBand)),
        h('div', { class: 'opts' },
          row('Bands', 'At most', ctl.maxBands),
          row('Max boost / cut', null, unit(ctl.maxBoost, 'dB'), unit(ctl.maxCut, 'dB')),
          row('Band 1 / 5 type', null, ctl.band1Type, ctl.band5Type),
          row('Also use Bass and Treble', 'Needs "B/T load with EQ preset" on the output', ctl.useBT))),
      h('div', { class: 'inline', style: { justifyContent: 'flex-end', marginTop: '8px' } }, resetOpts));
    const syncOpts = () => { for (const c of Object.values(ctl)) c.sync && c.sync(); };

    // result
    const cv = h('canvas', { class: 'curve hp-curve' });
    const legend = h('div', { class: 'hp-legend' },
      h('span', { class: 'k before' }, 'Deviation from target'), h('span', { class: 'k eq' }, 'EQ'), h('span', { class: 'k after' }, 'With EQ'));
    const status = h('div', { class: 'hint' });
    const bandsTbl = h('table', { class: 'tbl hp-bands' });
    const nameIn = h('input', { type: 'text', maxlength: 14, placeholder: 'Preset name', style: { width: '150px' }, 'aria-label': 'Preset name', oninput: () => { st.name = nameIn.value; st.nameAuto = false; } });
    const outSel = h('select', { 'aria-label': 'Output' }, X.outputs().filter((c) => !S.device.ready || X.has(X.key(c.addr, 12))).map((c) => h('option', { value: c.key, text: X.chName(c) })));
    outSel.value = X.CH.ph12 && (!S.device.ready || X.has(X.key(X.CH.ph12.addr, 12))) ? 'ph12' : outSel.options[0] && outSel.options[0].value;
    const actions = h('div', { class: 'inline hp-actions' },
      nameIn,
      h('button', { class: 'btn', title: 'Open the result in the EQ preset editor; nothing is sent', onclick: () => withResult(toEditor) }, 'Edit as preset'),
      h('span', { class: 'lab' }, 'on'), outSel,
      h('button', { class: 'btn needs-unit', title: 'Replace the EQ of this output with the result (Manual EQ, not stored)', onclick: () => withResult(apply) }, 'Apply'),
      h('button', { class: 'btn needs-unit', onclick: () => withResult(store) }, 'Store as preset…'),
      h('button', { class: 'btn', onclick: () => withResult(saveFile) }, 'Save file…'));
    const result = h('section', { class: 'panel' },
      h('div', { class: 'ph' }, h('h2', null, 'Result'), h('div', { class: 'grow' }), legend),
      h('div', { class: 'curve-wrap' }, cv),
      h('div', { class: 'hp-result-body' }, bandsTbl, h('div', null, status)),
      actions);

    const left = h('section', { class: 'panel hp-pick' },
      h('div', { class: 'ph' }, h('h2', null, 'Headphone'), h('div', { class: 'grow' }), formSeg),
      h('div', { class: 'inline', style: { flexWrap: 'nowrap' } }, search, ownFile),
      listBox, count);
    const tgt = h('section', { class: 'panel' },
      h('h2', null, 'Target'),
      h('div', { class: 'inline' }, targetSel, targetAutoBtn, targetFile),
      targetNote,
      h('p', { class: 'hint' }, 'Measurements and targets: AutoEq (oratory1990 measurements), from ADI-2-Pi. The fit is the same as in ADI-2-Pi\'s headphone EQ.'));

    root.append(h('div', { class: 'hp-top' }, left, h('div', { class: 'hp-col' }, tgt, options)), result);

    function visibleList() {
      if (!st.index) return [];
      const q = st.search.trim().toLowerCase().split(/\s+/).filter(Boolean);
      return st.index.headphones.filter((x) => (st.form === 'all' || x.form === st.form || (st.form === 'in-ear' && x.form === 'earbud')) && q.every((w) => x.name.toLowerCase().includes(w)));
    }
    function renderList() {
      const list = visibleList();
      listBox.replaceChildren(...list.map((x) => h('div', {
        class: 'hp-item', role: 'option', 'aria-selected': st.hp && st.hp.kind === 'repo' && st.hp.path === x.path ? 'true' : 'false',
        onclick: () => { st.hp = { kind: 'repo', ...x }; if (st.targetAuto) autoTarget(); renderList(); renderTarget(); save(st); run(); },
      }, h('span', null, x.name), h('span', { class: 'lab' }, x.form))));
      if (st.hp && st.hp.kind === 'file') listBox.prepend(h('div', { class: 'hp-item', role: 'option', 'aria-selected': 'true' }, h('span', null, st.hp.name), h('span', { class: 'lab' }, 'file')));
      count.textContent = st.index ? `${list.length} of ${st.index.headphones.length} headphones` : '';
    }
    function autoTarget() {
      if (!st.index) return;
      const form = (st.hp && st.hp.form) || 'over-ear';
      const want = form === 'over-ear' ? 'Harman over-ear 2018' : 'Harman in-ear 2019';
      const t = st.index.targets.find((x) => x.name === want) || st.index.targets[0];
      st.target = { kind: 'repo', ...t };
    }
    function renderTarget() {
      if (!st.index) return;
      targetSel.replaceChildren(...st.index.targets.map((t) => h('option', { value: t.path, text: t.name })));
      if (st.target && st.target.kind === 'file') {
        targetSel.prepend(h('option', { value: '', text: `${st.target.name} (file)` }));
        targetSel.value = '';
      } else if (st.target) targetSel.value = st.target.path;
      targetNote.textContent = st.targetAuto ? 'Chosen by headphone type.' : '';
    }

    async function csv(path) {
      if (!csvCache.has(path)) csvCache.set(path, invoke('eq_file', { path }).then((t) => C.parseResponse(t)));
      return csvCache.get(path);
    }

    let pending = null;
    async function run() {
      if (!st.hp || !st.target || !st.opt) return;
      const my = {};
      pending = my;
      status.textContent = 'Fitting…';
      status.className = 'hint';
      try {
        await core();
        const meas = st.hp.kind === 'file' ? st.hp.data : await csv(st.hp.path);
        const target = st.target.kind === 'file' ? st.target.data : await csv(st.target.path);
        const opt = { ...C.DEFAULTS, ...st.opt, device: 'adi2', app: 'adi2' };
        const t0 = performance.now();
        const r = await runFit(meas, target, opt);
        if (pending !== my) return;
        const ev = C.evaluate(r.an, r.fit.filters, opt);
        st.result = { an: r.an, filters: r.fit.filters, eq: ev.eq, stats: C.stats(r.an, ev.eq), ms: Math.round(performance.now() - t0), hpName: st.hp.name };
        if (window.HP.log) window.HP.log(`fit ${st.hp.name}: ${st.result.ms} ms, ${workerOk ? 'worker' : 'main thread'}`);
        if (!st.name || st.nameAuto) { st.name = st.hp.name.slice(0, 14); st.nameAuto = true; }
        drawResult();
      } catch (e) {
        if (pending !== my) return;
        st.result = null;
        status.textContent = e.message || String(e);
        status.className = 'hint bad';
        drawResult();
      }
    }

    function drawResult() {
      nameIn.value = st.name || '';
      const r = st.result;
      if (!r) { bandsTbl.replaceChildren(); drawChart(); return; }
      const fmt = (v, d = 1) => (v > 0 ? '+' : '') + v.toFixed(d).replace('-', '−');
      bandsTbl.replaceChildren(
        h('tr', null, ['Band', 'Type', 'Freq', 'Gain', 'Q'].map((t) => h('td', null, t))),
        ...r.filters.map((f) => h('tr', null,
          h('td', null, f.label), h('td', null, { peak: 'Peak', lowshelf: 'Low shelf', highshelf: 'High shelf' }[f.type] || f.type),
          h('td', { class: 'num' }, `${X.fmtHz(f.f)} Hz`), h('td', { class: 'num' }, `${fmt(f.gain)} dB`), h('td', { class: 'num' }, f.q.toFixed(1)))));
      if (!r.filters.length) bandsTbl.append(h('tr', null, h('td', { colspan: 5 }, 'No correction needed with these settings.')));
      const s2 = r.stats;
      status.className = 'hint';
      status.replaceChildren(
        h('div', null, `${r.hpName}: deviation from the target ${s2.rmsBefore.toFixed(1)} dB → ${s2.rmsAfter.toFixed(1)} dB (RMS, within the corrected range).`),
        s2.maxBoost > 0.05 ? h('div', null, `Largest boost ${s2.maxBoost.toFixed(1)} dB: lower the volume by that much if loud passages distort.`) : null);
      drawChart();
    }

    function drawChart() {
      const dpr = window.devicePixelRatio || 1;
      const W = cv.clientWidth, H = cv.clientHeight;
      if (!W || !H) return;
      cv.width = Math.round(W * dpr); cv.height = Math.round(H * dpr);
      const g = cv.getContext('2d');
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      g.clearRect(0, 0, W, H);
      const F0 = 20, F1 = 20000, DB = 15;
      const x = (f) => (Math.log10(f / F0) / Math.log10(F1 / F0)) * W;
      const y = (db) => H / 2 - (db / DB) * (H / 2 - 8);
      g.font = '10px system-ui, sans-serif';
      g.lineWidth = 1;
      for (const f of [30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000]) {
        g.strokeStyle = [100, 1000, 10000].includes(f) ? X.cssColor('var(--grid-major)') : X.cssColor('var(--grid)');
        g.beginPath(); g.moveTo(Math.round(x(f)) + 0.5, 0); g.lineTo(Math.round(x(f)) + 0.5, H); g.stroke();
      }
      g.fillStyle = X.cssColor('var(--faint)');
      for (const [f, t] of [[100, '100'], [1000, '1k'], [10000, '10k']]) g.fillText(t, x(f) + 3, H - 4);
      for (const db of [-12, -6, 0, 6, 12]) {
        g.strokeStyle = db === 0 ? X.cssColor('var(--grid-zero)') : X.cssColor('var(--grid)');
        g.beginPath(); g.moveTo(0, Math.round(y(db)) + 0.5); g.lineTo(W, Math.round(y(db)) + 0.5); g.stroke();
        g.fillText(db > 0 ? `+${db}` : String(db), 4, y(db) - 3);
      }
      const r = st.result;
      if (!r) return;
      const grid = r.an.grid;
      const line = (vals, color, width, dash) => {
        g.beginPath();
        for (let i = 0; i < grid.length; i++) {
          const v = Math.max(-DB - 5, Math.min(DB + 5, vals(i)));
          if (i === 0) g.moveTo(x(grid[i]), y(v)); else g.lineTo(x(grid[i]), y(v));
        }
        g.lineWidth = width; g.strokeStyle = color; g.setLineDash(dash || []); g.stroke(); g.setLineDash([]);
      };
      const err = r.an.errorSmoothed, roll = r.an.roll;
      line((i) => err[i] + roll[i], X.cssColor('var(--faint)'), 1.6);
      line((i) => r.eq[i], X.cssColor('var(--accent)'), 2.2);
      line((i) => err[i] + roll[i] + r.eq[i], X.cssColor('var(--on)'), 1.8);
      if (r.an.fHigh < F1) {
        g.fillStyle = X.cssColor('var(--shadow)');
        g.globalAlpha = 0.35;
        g.fillRect(x(r.an.fHigh), 0, W - x(r.an.fHigh), H);
        g.globalAlpha = 1;
      }
    }
    new ResizeObserver(() => requestAnimationFrame(drawChart)).observe(cv);
    cv.addEventListener('themechange', drawChart);

    function withResult(fn) {
      if (!st.result) { toast('No result yet'); return; }
      fn(st.result);
    }
    const presetOf = (r, n = 1) => {
      const { L } = toPresetValues(r.filters, S.schema.eqDefaults.values);
      return { n, name: (nameIn.value || r.hpName).trim().slice(0, 14), empty: false, dual: false, L, R: {} };
    };
    function toEditor(r) {
      // into the first empty slot
      const free = S.presets.find((p) => p.empty === true);
      const n = free ? free.n : S.editor ? S.editor.n : 1;
      const p = presetOf(r, n);
      const R = {};
      for (let i = 3; i <= 19; i++) R[i] = p.L[i];
      S.editor = { n, name: p.name, dual: false, L: p.L, R, base: null };
      X.go('presets');
    }
    async function apply(r) {
      const c = X.CH[outSel.value];
      if (!c || !X.guard()) return;
      if (!(await X.confirmDlg(`Apply to ${X.chName(c)}?`, `Replaces the EQ of ${X.chName(c)} with this result (as Manual EQ; nothing is stored). Store the current EQ as a preset first if you want to keep it.`, 'Apply'))) return;
      const { L, bt } = toPresetValues(r.filters, S.schema.eqDefaults.values);
      const items = [];
      for (let i = 3; i <= 19; i++) items.push([X.key(c.eq, i), L[i]]);
      for (let i = 21; i <= 26; i++) items.push([X.key(c.eq, i), L[i]]);
      items.push([X.key(c.addr, S.schema.params.output.find((p) => p.id === 'dualeq').idx), 0], [X.key(c.eq, 2), 1]);
      if (bt) items.push([X.key(c.eq, 20), 1]);
      for (const [k, v] of items) S.p.set(k, v);
      invoke('set', { items }).then(() => toast(`Applied to ${X.chName(c)}`)).catch((e) => toast(e, true));
    }
    function store(r) {
      if (!X.guard()) return;
      const free = S.presets.find((p) => p.empty === true);
      X.importPresets(null, [presetOf(r, free ? free.n : 1)], r.hpName);
    }
    function saveFile(r) {
      const p = presetOf(r, 1);
      invoke('write_presets', { presets: [p] })
        .then((text) => invoke('save_file', { title: 'Save EQ preset', name: `${p.name || 'EQ'}.adieqpr`, ext: ['adieqpr'], label: 'ADI-2 EQ preset', text }))
        .then((path) => { if (path) toast(`Saved ${path}`); })
        .catch((e) => toast(e, true));
    }

    // start
    (async () => {
      try {
        const K = await core();
        if (!st.opt) st.opt = { ...K.DEFAULTS, ...Object.fromEntries(Object.entries(st.savedOpt).filter(([k]) => OPT_KEYS.includes(k))) };
        syncOpts();
        if (!st.index) st.index = JSON.parse(await invoke('eq_index'));
        if (!st.hp) {
          const want = st.savedHp || '';
          const def = st.index.headphones.find((x) => x.path === want) || st.index.headphones.find((x) => x.name === 'Sennheiser HD 650') || st.index.headphones[0];
          st.hp = { kind: 'repo', ...def };
        }
        if (!st.target) {
          const t = st.savedTarget && st.index.targets.find((x) => x.path === st.savedTarget);
          if (t) st.target = { kind: 'repo', ...t }; else { st.targetAuto = true; autoTarget(); }
        }
        renderList();
        renderTarget();
        const selEl = listBox.querySelector('[aria-selected="true"]');
        if (selEl) requestAnimationFrame(() => { listBox.scrollTop = selEl.offsetTop - listBox.offsetTop - 60; });
        if (st.result) drawResult(); else run();
      } catch (e) {
        status.textContent = e.message || String(e);
        status.className = 'hint bad';
      }
    })();
    return root;
  }

  window.HP = { page };
})();
