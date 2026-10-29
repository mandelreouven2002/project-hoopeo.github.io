/* The scripts of the website: the theme button, the tabs of the Install page (opening the one of the visitor's system), and
   the glossary popover. Nothing is sent anywhere. */
(function () {
  var root = document.documentElement;
  root.classList.add('js');

  function read() { try { return localStorage.getItem('hoopeo-theme'); } catch (e) { return null; } }
  function write(v) { try { localStorage.setItem('hoopeo-theme', v); } catch (e) { /* private window: fine */ } }
  function effective() {
    var t = root.getAttribute('data-theme');
    if (t) return t;
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  var saved = read();
  if (saved === 'light' || saved === 'dark') root.setAttribute('data-theme', saved);

  // The visitor's system: mac, windows or linux (nothing is sent anywhere)
  var plat = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent || '').toLowerCase();
  var os = /mac/.test(plat) ? 'mac' : /win/.test(plat) ? 'windows' : /linux|x11|cros/.test(plat) ? 'linux' : '';
  if (os) root.setAttribute('data-os', os);

  var btn = document.getElementById('theme-btn');
  if (btn) {
    var label = function () {
      var dark = effective() === 'dark';
      var text = dark ? 'Switch to light theme' : 'Switch to dark theme';
      btn.setAttribute('aria-label', text);
      btn.setAttribute('title', text);
      btn.setAttribute('aria-pressed', dark ? 'true' : 'false');
    };
    label();
    btn.addEventListener('click', function () {
      var next = effective() === 'dark' ? 'light' : 'dark';
      root.setAttribute('data-theme', next);
      write(next);
      label();
    });
  }

  // Tabs: each tab is a link to a panel, so without the script every panel is simply shown.
  document.querySelectorAll('.tabs').forEach(function (box) {
    var tabs = Array.prototype.slice.call(box.querySelectorAll('[role="tab"]'));
    var panels = tabs.map(function (t) { return document.getElementById(t.getAttribute('href').slice(1)); });
    function show(i, focus) {
      tabs.forEach(function (t, j) {
        t.setAttribute('aria-selected', i === j ? 'true' : 'false');
        t.tabIndex = i === j ? 0 : -1;
        if (panels[j]) panels[j].hidden = i !== j;
      });
      if (focus) tabs[i].focus();
    }
    function fromHash() {
      var h = location.hash.slice(1), i = panels.findIndex(function (p) { return p && p.id === h; });
      return i < 0 ? -1 : i;
    }
    // no panel named in the address: the tab of the visitor's system, else the first
    function initial() {
      var i = fromHash();
      if (i >= 0) return i;
      var m = tabs.findIndex(function (t) { return os && t.getAttribute('data-os') === os; });
      return m < 0 ? 0 : m;
    }
    tabs.forEach(function (t, i) {
      t.addEventListener('click', function (e) { e.preventDefault(); show(i); history.replaceState(null, '', t.getAttribute('href')); });
      t.addEventListener('keydown', function (e) {
        if (e.key === 'ArrowRight') { e.preventDefault(); show((i + 1) % tabs.length, true); }
        if (e.key === 'ArrowLeft') { e.preventDefault(); show((i + tabs.length - 1) % tabs.length, true); }
      });
    });
    show(initial());
    window.addEventListener('hashchange', function () { var i = fromHash(); if (i >= 0) show(i); });
  });
})();

/* The glossary popover. Nothing is sent anywhere. */
(function () {
  // Glossary terms: <a class="term" href=".../glossary/#anchor" data-def="One-line definition">term</a>
  var pop = null;
  function hide() { if (pop) { pop.remove(); pop = null; } }
  function show(a) {
    hide();
    pop = document.createElement('div');
    pop.className = 'term-pop';
    pop.setAttribute('role', 'tooltip');
    var name = a.getAttribute('data-term') || a.textContent;
    pop.innerHTML = '<strong></strong><span></span><small>Open in the glossary</small>';
    pop.querySelector('strong').textContent = name;
    pop.querySelector('span').textContent = a.getAttribute('data-def');
    document.body.appendChild(pop);
    var r = a.getBoundingClientRect();
    var left = Math.min(window.scrollX + r.left, window.scrollX + document.documentElement.clientWidth - pop.offsetWidth - 12);
    pop.style.left = Math.max(window.scrollX + 12, left) + 'px';
    pop.style.top = (window.scrollY + r.bottom + 8) + 'px';
  }
  document.querySelectorAll('a.term[data-def]').forEach(function (a) {
    a.addEventListener('mouseenter', function () { show(a); });
    a.addEventListener('focus', function () { show(a); });
    a.addEventListener('mouseleave', hide);
    a.addEventListener('blur', hide);
  });
  document.addEventListener('keydown', function (e) { if (e.key === 'Escape') hide(); });
})();

/* Deep links also open the relevant in-page help or collapsed explanation. */
(function () {
  function openTarget() {
    let id;
    try { id = decodeURIComponent(location.hash.slice(1)); } catch { return; }
    const target = id && document.getElementById(id);
    if (!target) return;
    let node = target;
    let opened = false;
    while (node) {
      if (node.tagName === 'DETAILS' && !node.open) { node.open = true; opened = true; }
      node = node.parentElement;
    }
    if (opened) requestAnimationFrame(() => target.scrollIntoView({ block: 'start' }));
  }
  openTarget();
  window.addEventListener('hashchange', openTarget);
})();

/* Developer edition: navigation, code copying and a local documentation index. */
(function () {
  'use strict';
  const menu = document.querySelector('.menu-toggle');
  const nav = document.getElementById('main-nav');
  if (menu && nav) {
    menu.addEventListener('click', () => {
      const open = menu.getAttribute('aria-expanded') !== 'true';
      menu.setAttribute('aria-expanded', String(open));
      nav.classList.toggle('is-open', open);
    });
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && menu.getAttribute('aria-expanded') === 'true') {
        menu.setAttribute('aria-expanded', 'false'); nav.classList.remove('is-open'); menu.focus();
      }
    });
  }
  document.querySelectorAll('main pre').forEach((pre) => {
    if (pre.closest('.proof-window') || pre.closest('.code-wrap') || pre.closest('.sql-editor')) return;
    const code = pre.querySelector('code');
    if (!code) return;
    const wrap = document.createElement('div'); wrap.className = 'code-wrap';
    pre.before(wrap); wrap.append(pre);
    const lang = document.createElement('span'); lang.className = 'code-language';
    lang.textContent = (code.className.match(/language-([\w-]+)/) || [null, 'code'])[1];
    const button = document.createElement('button'); button.type = 'button'; button.className = 'code-copy';
    button.textContent = 'Copy'; button.setAttribute('aria-label', 'Copy code');
    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(code.textContent);
        button.textContent = 'Copied!';
      } catch (_) {
        const selection = window.getSelection(); const range = document.createRange();
        range.selectNodeContents(code); selection.removeAllRanges(); selection.addRange(range);
        button.textContent = 'Selected · copy manually';
      }
      setTimeout(() => { button.textContent = 'Copy'; }, 1800);
    });
    wrap.append(lang, button);
  });
  const launchers = [...document.querySelectorAll('.search-open')];
  if (!launchers.length) return;
  const dialog = document.createElement('dialog'); dialog.className = 'search-dialog';
  dialog.setAttribute('aria-label', 'Search documentation');
  dialog.innerHTML = '<div class="search-head"><input type="search" aria-label="Search documentation" placeholder="Search concepts, SQL, setup…"><button type="button" class="search-close" aria-label="Close search">Esc</button></div><div class="search-results" aria-live="polite"></div>';
  document.body.append(dialog);
  const input = dialog.querySelector('input'); const results = dialog.querySelector('.search-results');
  let index = null; let loading = null;
  const script = [...document.scripts].find((script) => script.src && new URL(script.src).pathname.endsWith('/assets/js/site.js'));
  const indexURL = new URL('../../search-index.json', script.src);
  function notice(message) { const p = document.createElement('p'); p.className = 'search-empty'; p.textContent = message; results.replaceChildren(p); }
  function search() {
    if (!index) return;
    const query = input.value.trim().toLowerCase();
    if (!query) { notice('Try “protected”, “Python”, “TLS” or “GDPR”.'); return; }
    const words = query.split(/\s+/);
    const matches = index.map((page) => ({...page, score: words.reduce((score, word) => score + (page.title.toLowerCase().includes(word) ? 10 : 0) + (page.text.toLowerCase().includes(word) ? 1 : 0), 0)}))
      .filter((page) => words.every((word) => (page.title + ' ' + page.text).toLowerCase().includes(word)))
      .sort((a, b) => b.score - a.score).slice(0, 12);
    results.replaceChildren();
    const count = document.createElement('p'); count.className = 'search-count'; count.textContent = `${matches.length} result${matches.length === 1 ? '' : 's'}`; results.append(count);
    if (!matches.length) { notice('No matching pages. Try a shorter search.'); return; }
    matches.forEach((page) => {
      const a = document.createElement('a'); a.className = 'search-result'; a.href = page.url;
      const title = document.createElement('strong'); title.textContent = page.title;
      const excerpt = document.createElement('p');
      const at = Math.max(0, page.text.toLowerCase().indexOf(words[0]) - 65);
      excerpt.textContent = (at ? '…' : '') + page.text.slice(at, at + 180).replace(/\s+/g, ' ') + '…';
      a.append(title, excerpt); results.append(a);
    });
  }
  async function open() {
    if (!dialog.open) dialog.showModal(); input.focus();
    if (!index) {
      notice('Loading documentation…');
      try {
        if (!loading) loading = fetch(indexURL).then((response) => { if (!response.ok) throw new Error('Index unavailable'); return response.json(); });
        index = await loading;
      } catch (_) { loading = null; notice('Search could not load. Close and try again, or browse Docs from the navigation.'); return; }
    }
    search();
  }
  launchers.forEach((button) => button.addEventListener('click', open));
  input.addEventListener('input', search);
  dialog.querySelector('.search-close').addEventListener('click', () => dialog.close());
  dialog.addEventListener('click', (event) => { if (event.target === dialog) { const r = dialog.getBoundingClientRect(); if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) dialog.close(); } });
  document.addEventListener('keydown', (event) => {
    if ((event.key === '/' && !/INPUT|TEXTAREA|SELECT/.test(document.activeElement.tagName) && !document.activeElement.isContentEditable) || ((event.metaKey || event.ctrlKey) && event.key === 'k')) { event.preventDefault(); open(); }
  });
})();
