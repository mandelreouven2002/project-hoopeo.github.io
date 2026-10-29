/* Render the book's existing Mermaid sources with a local, licensed bundle. */
(function () {
  const nodes = [...document.querySelectorAll('.book-diagram .mermaid')];
  if (!nodes.length || !window.mermaid) return;
  const definitions = nodes.map((node) => node.textContent);
  let running = false;
  let pending = false;
  let firstRender = true;
  let revision = 0;
  let readerInteracted = false;
  ['wheel', 'touchstart', 'keydown'].forEach((event) => window.addEventListener(event, () => { readerInteracted = true; }, { once: true, passive: true }));
  function dark() {
    return document.documentElement.dataset.theme === 'dark' ||
      (!document.documentElement.dataset.theme && matchMedia('(prefers-color-scheme: dark)').matches);
  }
  async function render() {
    if (running) { pending = true; return; }
    running = true;
    const renderRevision = ++revision;
    await document.fonts.ready;
    window.mermaid.initialize({
      startOnLoad: false, securityLevel: 'strict', theme: dark() ? 'dark' : 'default',
      fontFamily: 'IBM Plex Sans, sans-serif',
      flowchart: { htmlLabels: false, useMaxWidth: true },
      sequence: { useMaxWidth: true, wrap: true },
    });
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      try {
        const result = await window.mermaid.render('book-diagram-' + renderRevision + '-' + i, definitions[i]);
        node.innerHTML = result.svg;
        node.dataset.rendered = 'true';
        node.removeAttribute('data-error');
        const svg = node.querySelector('svg');
        svg.setAttribute('role', 'img');
        svg.setAttribute('aria-label', node.closest('figure').querySelector('figcaption').textContent);
      } catch (error) {
        node.textContent = 'The diagram could not be rendered. Its complete text is available below.';
        node.dataset.error = 'true';
        console.error('Book diagram failed:', error);
      }
    }
    running = false;
    if (firstRender && location.hash && !readerInteracted) {
      let id;
      try { id = decodeURIComponent(location.hash.slice(1)); } catch { id = ''; }
      document.getElementById(id)?.scrollIntoView({ block: 'start' });
    }
    firstRender = false;
    if (pending) { pending = false; render(); }
  }
  render();
  new MutationObserver(render).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', render);
})();
