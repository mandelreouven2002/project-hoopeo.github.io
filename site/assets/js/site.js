/* The only script of the website: the theme button and the tabs of the Install page. Nothing is sent anywhere. */
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
      return i < 0 ? 0 : i;
    }
    tabs.forEach(function (t, i) {
      t.addEventListener('click', function (e) { e.preventDefault(); show(i); history.replaceState(null, '', t.getAttribute('href')); });
      t.addEventListener('keydown', function (e) {
        if (e.key === 'ArrowRight') { e.preventDefault(); show((i + 1) % tabs.length, true); }
        if (e.key === 'ArrowLeft') { e.preventDefault(); show((i + tabs.length - 1) % tabs.length, true); }
      });
    });
    show(fromHash());
    window.addEventListener('hashchange', function () { show(fromHash()); });
  });
})();
