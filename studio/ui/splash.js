/* Hoopeo Studio splash screen. */
  (function () {
    var t = window.__TAURI__;
    if (t && t.core) {
      t.core.invoke('app_info').then(function (i) { document.getElementById('ver').textContent = 'Version ' + i.version; }).catch(function () {});
    }
    var steps = ['Starting DEP Engine...', 'Loading connections...', 'Preparing the workspace...'];
    var n = 0;
    setInterval(function () { n = Math.min(n + 1, steps.length - 1); document.getElementById('status').textContent = steps[n]; }, 500);
  })();
