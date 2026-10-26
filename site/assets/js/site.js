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
