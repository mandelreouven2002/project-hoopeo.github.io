/* Local syntax highlighting. Text is escaped by highlight.js, never executed. */
(function () {
  'use strict';
  if (!window.hljs) return;
  const sql = hljs.getLanguage('sql');
  if (sql && sql.keywords) {
    const extra='protected match domain discloses links reveal endpoint operational masked_read privacy matrix limit'.split(' ');
    const existing=sql.keywords.keyword;
    sql.keywords.keyword=Array.isArray(existing)?existing.concat(extra):existing+' '+extra.join(' ');
  }
  function highlight(code) {
    const declared = (code.className.match(/language-([\w-]+)/) || [])[1];
    const language = ({sh: 'bash', js: 'javascript', depsql: 'sql'})[declared] || declared;
    if (!language || !hljs.getLanguage(language) || ['text', 'plaintext'].includes(language)) return;
    const source = code.textContent;
    code.innerHTML = hljs.highlight(source, {language, ignoreIllegals: true}).value;
    code.classList.add('hljs');
  }
  window.HoopeoCode = {
    highlight,
    sql(source) { return hljs.highlight(source, {language:'sql', ignoreIllegals:true}).value; }
  };
  document.querySelectorAll('pre code').forEach(highlight);
})();

/* Keep the Playground's editable SQL colored without changing its text or query behavior. */
(function () {
  'use strict';
  const textarea = document.getElementById('sql');
  if (!textarea || !window.HoopeoCode) return;
  const editor = document.createElement('div'); editor.className='sql-editor';
  const colors=document.createElement('pre');colors.className='sql-colors';colors.setAttribute('aria-hidden','true');
  const code=document.createElement('code');code.className='language-sql hljs';colors.append(code);
  textarea.before(editor);editor.append(colors,textarea);
  textarea.setAttribute('spellcheck','false');textarea.setAttribute('wrap','off');
  function paint() {code.innerHTML=HoopeoCode.sql(textarea.value+'\n');colors.scrollTop=textarea.scrollTop;colors.scrollLeft=textarea.scrollLeft;}
  textarea.addEventListener('input',paint);
  textarea.addEventListener('scroll',()=>{colors.scrollTop=textarea.scrollTop;colors.scrollLeft=textarea.scrollLeft;});
  document.addEventListener('hoopeo:querychange',paint);paint();
})();
