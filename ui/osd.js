'use strict';
window.__TAURI__.event.listen('osd', (e) => {
  const i = e.payload;
  document.getElementById('t').textContent = i.title;
  document.getElementById('v').textContent = i.value;
  document.getElementById('bar').style.visibility = i.frac === null || i.frac === undefined ? 'hidden' : 'visible';
  document.getElementById('fill').style.width = `${Math.round((i.frac || 0) * 100)}%`;
  document.getElementById('box').classList.toggle('muted', !!i.muted);
});
