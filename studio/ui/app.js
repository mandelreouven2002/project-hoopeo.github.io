/* Hoopeo Studio, the main window.
 *
 * Every command is in the native menu bar; a menu choice arrives here as a "menu" event with
 * the item id (see src/main.rs). The window has no toolbar: the object browser on the left, SQL
 * windows on the right, and below them the Output pane (a log of every statement with its
 * result, its timing and the DEP notes) next to the result grids.
 */
(function () {
'use strict';
const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (s, r) => (r || document).querySelector(s);
const $$ = (s, r) => Array.from((r || document).querySelectorAll(s));
const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const store = {
  get(k, d) { try { const v = localStorage.getItem('hoopeo-studio.' + k); return v === null ? d : JSON.parse(v); } catch (e) { return d; } },
  set(k, v) { try { localStorage.setItem('hoopeo-studio.' + k, JSON.stringify(v)); } catch (e) { /* storage unavailable */ } }
};
const ERR_INFO = (window.DEP_REFERENCE && window.DEP_REFERENCE.errors) || {};

const WELCOME = `-- Hoopeo Studio
-- F8 executes the window (or the selection), Ctrl+Enter the statement at the cursor,
-- F5 shows the plan after the DEP rewrite. The Output pane shows each result and its DEP notes.

SELECT full_name, national_id, health_fund
FROM patients
WHERE national_id = '123456782';
`;

/* ------------------------------------------------------------------ state */

let conn = null;            // ConnInfo of the active connection
let objects = null;         // last browse() result
let editor = null;
let docs = store.get('docs', null) || [{ id: 1, title: 'SQL 1', text: WELCOME, path: null, saved: WELCOME }];
let activeDoc = store.get('activeDoc', docs[0].id);
let panes = [{ id: 'output', title: 'Output' }];   // bottom tabs
let activePane = 'output';
let outputEntries = [];     // the Output log
let lastNotes = [];
let fontScale = store.get('fontScale', 1);
let selectedNode = null;
let txnState = null;        // null, 'open' or 'failed': the transaction of the active connection
let runSeq = 0;             // numbers each execution, for the Output tab's badges

function setTxnUI() {
  const el = $('#stTxn');
  el.textContent = txnState === 'open' ? 'Transaction open' : txnState === 'failed' ? 'Transaction failed: Rollback' : '';
  el.className = 'st ' + (txnState ? 'txn ' + txnState : '');
}

/* ------------------------------------------------------------------ errors and messages */

function parseErr(e) {
  if (e && typeof e === 'object') return e;
  try { return JSON.parse(e); } catch (x) { return { code: null, sqlstate: '', message: String(e), hint: null }; }
}
function errText(e) {
  const j = parseErr(e);
  return (j.code ? j.code + ': ' : (j.sqlstate ? 'ERROR ' + j.sqlstate + ': ' : '')) + j.message + (j.hint ? '\n' + j.hint : '');
}

function messageBox(title, html, buttons) {
  buttons = buttons || [{ label: 'OK', value: 'ok', default: true }];
  return new Promise((resolve) => {
    const d = $('#dlgMsg');
    $('#dlgMsgTitle').textContent = title;
    $('#msgText').innerHTML = html;
    const bar = $('#msgButtons');
    bar.innerHTML = '<span class="spacer"></span>' + buttons.map((b, i) => `<button type="button" class="btn${b.default ? ' default' : ''}" data-i="${i}">${esc(b.label)}</button>`).join('');
    const done = (v) => { d.close(); resolve(v); };
    $$('button', bar).forEach((b) => { b.onclick = () => done(buttons[Number(b.dataset.i)].value); });
    d.oncancel = (ev) => { ev.preventDefault(); done('cancel'); };
    d.showModal();
    const def = $('.btn.default', bar); if (def) def.focus();
  });
}

function openDialog(d) {
  d.showModal();
  $$('[data-close]', d).forEach((b) => { b.onclick = () => d.close(); });
}

/* ------------------------------------------------------------------ status and connection UI */

function setConnUI() {
  const c = $('#stConn');
  c.classList.toggle('on', !!conn);
  c.textContent = conn ? conn.name : 'Not connected';
  $('#stRole').textContent = conn ? `${conn.role}@${conn.endpoint}` : '';
  $('#stPath').textContent = conn ? conn.path : '';
  $('#stWhere').textContent = conn ? conn.location : '';
  $('#stTls').textContent = conn && conn.kind === 'server' ? (conn.tls === 'disable' ? 'TLS off' : 'TLS ' + conn.tls) : '';
  $('#stServe').textContent = conn && conn.serving ? 'Serving on ' + conn.serving : '';
  $('#browserConn').textContent = conn ? `${conn.role}@${conn.endpoint}` : 'Not connected';
  document.title = conn ? `Hoopeo Studio - ${conn.name} (${conn.role}@${conn.endpoint})` : 'Hoopeo Studio';
}

function kindLabel(p) {
  if (p.kind === 'server') return `${p.user}@${p.host || '127.0.0.1'}:${p.port}/${p.endpoint}${p.tls && p.tls !== 'disable' ? ' - TLS' : ''}`;
  if (p.kind === 'local') return `${p.user} - ${p.dataDir || 'default data directory'}${p.servePort ? ' - port ' + p.servePort : ''}`;
  return `${p.user}@${p.endpoint} - in memory`;
}

/* ------------------------------------------------------------------ logon dialog */

let profiles = [];
let selectedProfile = null;

async function openConnect(kind) {
  profiles = await invoke('list_profiles');
  renderProfiles();
  const wanted = kind ? profiles.find((p) => p.kind === kind) : profiles.find((p) => p.name === store.get('lastProfile', ''));
  await selectProfile(wanted || (kind ? blankProfile(kind) : profiles[0]));
  hideFormMsgs();
  $('#dlgConnectTitle').textContent = kind === 'local' ? 'Open Local Database' : kind === 'server' ? 'Connect to DEP Server' : 'Connect';
  openDialog($('#dlgConnect'));
  (selectedProfile.kind === 'server' ? $('#pfPassword') : $('#btnDoConnect')).focus();
}

function blankProfile(kind) {
  return { name: kind === 'local' ? 'Local database' : 'DEP server', kind, host: kind === 'server' ? '127.0.0.1' : '', port: 5433, user: 'admin', endpoint: 'operational', dataDir: '', tls: 'disable', caFile: '', servePort: null };
}

function renderProfiles() {
  const groups = [['server', 'DEP Engine servers'], ['local', 'Local databases'], ['memory', 'In memory']];
  let html = '';
  for (const [k, title] of groups) {
    const items = profiles.map((p, i) => [p, i]).filter(([p]) => p.kind === k);
    if (!items.length) continue;
    html += `<li class="sec">${esc(title)}</li>` + items.map(([p, i]) => `<li data-i="${i}" role="option" class="${selectedProfile && selectedProfile.name === p.name ? 'selected' : ''}">${esc(p.name)}<span class="k">${esc(kindLabel(p))}</span></li>`).join('');
  }
  $('#profileList').innerHTML = html;
  $$('#profileList li[data-i]').forEach((li) => {
    li.addEventListener('click', () => selectProfile(profiles[Number(li.dataset.i)]));
    li.addEventListener('dblclick', () => { selectProfile(profiles[Number(li.dataset.i)]); doConnect(); });
  });
}

async function selectProfile(p) {
  selectedProfile = p;
  $('#pfName').value = p.name || '';
  $('#pfKind').value = p.kind || 'server';
  $('#pfHost').value = p.host || '';
  $('#pfPort').value = p.port || 5433;
  $('#pfUser').value = p.user || 'admin';
  $('#pfEndpoint').value = p.endpoint || 'operational';
  $('#pfDataDir').value = p.dataDir || '';
  $('#pfDataDir').placeholder = await invoke('default_data_dir');
  $('#pfTls').value = p.tls || 'disable';
  $('#pfCaFile').value = p.caFile || '';
  $('#pfPassword').value = '';
  renderProfiles();
  kindChanged();
}

function kindChanged() {
  const k = $('#pfKind').value;
  $$('#dlgConnect [data-show]').forEach((el) => { el.hidden = el.dataset.show !== k; });
  $('#pfCaRow').hidden = k !== 'server' || $('#pfTls').value !== 'verify-full';
  $('#pfHint').textContent = {
    server: 'A DEP Engine server on this computer, another computer or the cloud, over the PostgreSQL wire protocol. The role is the user name; the endpoint decides the access path.',
    local: 'A data directory on this computer, opened by Studio itself. No server may be using it at the same time; to share it, serve it on a port (Session > Serve on Port).',
    memory: 'An empty database in memory, as admin. It resets when Studio quits.'
  }[k];
}

function formProfile() {
  return {
    name: $('#pfName').value.trim() || 'Connection',
    kind: $('#pfKind').value,
    host: $('#pfHost').value.trim(),
    port: Number($('#pfPort').value) || 5433,
    user: $('#pfUser').value.trim() || 'admin',
    endpoint: $('#pfEndpoint').value.trim() || 'operational',
    dataDir: $('#pfDataDir').value.trim(),
    tls: $('#pfTls').value,
    caFile: $('#pfCaFile').value.trim(),
    servePort: selectedProfile && selectedProfile.name === $('#pfName').value.trim() ? (selectedProfile.servePort || null) : null
  };
}

function hideFormMsgs() { $('#pfError').hidden = true; $('#pfOk').hidden = true; }
function formError(e) { hideFormMsgs(); $('#pfError').textContent = errText(e); $('#pfError').hidden = false; }

async function doConnect() {
  const p = formProfile();
  $('#btnDoConnect').disabled = true;
  try {
    const info = await invoke('connect', { profile: p, password: p.kind === 'server' ? $('#pfPassword').value : null, adminPassword: null });
    if (conn) await invoke('disconnect', { id: conn.id });
    conn = info;
    store.set('lastProfile', p.name);
    $('#dlgConnect').close();
    connected(p);
  } catch (e) {
    formError(e);
  } finally {
    $('#btnDoConnect').disabled = false;
  }
}

async function testConnection() {
  const p = formProfile();
  hideFormMsgs();
  $('#btnTestProfile').disabled = true;
  try {
    const info = await invoke('connect', { profile: p, password: p.kind === 'server' ? $('#pfPassword').value : null, adminPassword: null });
    await invoke('disconnect', { id: info.id });
    $('#pfOk').textContent = `Connected as ${info.role} through ${info.endpoint} (${info.path}), ${info.location}${info.kind === 'server' ? ', TLS ' + info.tls : ''}.`;
    $('#pfOk').hidden = false;
  } catch (e) {
    formError(e);
  } finally {
    $('#btnTestProfile').disabled = false;
  }
}

function connected(p) {
  txnState = null;
  setTxnUI();
  setConnUI();
  logInfo(`Connected to ${conn.name}: ${conn.role}@${conn.endpoint}, access path ${conn.path}, ${conn.location}${conn.kind === 'server' ? ', TLS ' + conn.tls : ''}${conn.serving ? ', serving on ' + conn.serving : ''}.`);
  refreshObjects();
}

async function connectProfile(profile, password, adminPassword) {
  const info = await invoke('connect', { profile, password: password || null, adminPassword: adminPassword || null });
  if (conn) await invoke('disconnect', { id: conn.id });
  conn = info;
  store.set('lastProfile', profile.name);
  connected(profile);
}

async function disconnect() {
  if (!conn) return;
  const name = conn.name;
  await invoke('disconnect', { id: conn.id });
  conn = null;
  objects = null;
  txnState = null;
  setTxnUI();
  setConnUI();
  renderTree();
  renderProps(null);
  logInfo(`Disconnected from ${name}.`);
}

/* ------------------------------------------------------------------ new local database, serving */

async function openNewLocal() {
  $('#nlError').hidden = true;
  $('#nlName').value = 'My database';
  $('#nlDir').value = await invoke('suggest_data_dir', { name: 'My database' });
  $('#nlServe').checked = false;
  $('#nlPassword').value = '';
  openDialog($('#dlgNewLocal'));
  $('#nlName').select();
}

async function createLocal() {
  const name = $('#nlName').value.trim() || 'Local database';
  const memory = $('input[name="nlStore"]:checked').value === 'memory';
  const serve = $('#nlServe').checked;
  const port = Number($('#nlPort').value);
  const pw = $('#nlPassword').value;
  const fail = (m) => { $('#nlError').textContent = errText(m); $('#nlError').hidden = false; };
  if (serve && !(port > 0 && port < 65536)) return fail('Enter a port between 1 and 65535.');
  if (serve && pw.length < 4) return fail('Choose a password for admin (at least 4 characters): every connection over the network authenticates.');
  const profile = { name, kind: memory ? 'memory' : 'local', host: '', port: 5433, user: 'admin', endpoint: 'operational', dataDir: memory ? '' : $('#nlDir').value.trim(), tls: 'disable', caFile: '', servePort: serve ? port : null };
  $('#btnCreateLocal').disabled = true;
  try {
    await connectProfile(profile, null, serve ? pw : null);
    if (!memory) await invoke('save_profile', { profile: Object.assign({}, profile, { servePort: null }) });
    $('#dlgNewLocal').close();
    if (serve) servingMessage();
  } catch (e) {
    fail(e);
  } finally {
    $('#btnCreateLocal').disabled = false;
  }
}

function openServe() {
  if (!needConn()) return;
  if (conn.kind === 'server') { messageBox('Serve on Port', 'This connection is to a DEP Engine server, which is already reachable on its own port.'); return; }
  $('#svError').hidden = true;
  $('#svIntro').textContent = `Serve "${conn.name}" over the PostgreSQL wire protocol while Studio keeps it open, so that DEP CLI, psql and other tools can connect to it.`;
  $('#svPassword').value = '';
  openDialog($('#dlgServe'));
  $('#svPort').focus();
}

async function doServe() {
  const port = Number($('#svPort').value);
  if (!(port > 0 && port < 65536)) { $('#svError').textContent = 'Enter a port between 1 and 65535.'; $('#svError').hidden = false; return; }
  try {
    conn = await invoke('serve_local', { id: conn.id, bind: $('#svBind').value, port, adminPassword: $('#svPassword').value || null });
    $('#dlgServe').close();
    setConnUI();
    logInfo(`Serving ${conn.name} on ${conn.serving}.`);
    servingMessage();
  } catch (e) {
    $('#svError').textContent = errText(e);
    $('#svError').hidden = false;
  }
}

function servingMessage() {
  const [host, port] = conn.serving.split(':');
  const h = host === '0.0.0.0' ? '<this computer>' : host;
  messageBox('Serving', `"${esc(conn.name)}" is served on ${esc(conn.serving)} while Studio keeps it open. Connect with:<pre>dep-cli -h ${esc(h)} -p ${esc(port)} -U admin -d operational\npsql    -h ${esc(h)} -p ${esc(port)} -U admin -d operational</pre>`);
}

async function stopServing() {
  if (!needConn() || !conn.serving) return;
  const was = conn.serving;
  conn = await invoke('stop_serving', { id: conn.id });
  setConnUI();
  logInfo(`Stopped serving on ${was}.`);
}

function needConn() {
  if (conn) return true;
  logInfo('Not connected. Use Session > Connect to DEP Server, Open Local Database, or New Local Database.');
  showPane('output');
  return false;
}

/* ------------------------------------------------------------------ object browser */

async function refreshObjects() {
  if (!conn) { objects = null; renderTree(); return; }
  try { objects = await invoke('browse', { id: conn.id }); } catch (e) { objects = null; }
  renderTree();
}

const openNodes = new Set(store.get('openNodes', ['tables', 'functions']));

function node(id, depth, ico, label, meta, opts) {
  opts = opts || {};
  const has = !!opts.children;
  const open = has && openNodes.has(id);
  return `<div class="node${selectedNode === id ? ' selected' : ''}" data-id="${esc(id)}" data-kind="${opts.kind || ''}" data-i="${opts.i == null ? '' : opts.i}" style="--depth:${depth}" role="treeitem"${has ? ` aria-expanded="${open}"` : ''}>` +
    `<span class="tw${has ? '' : ' leaf'}">${open ? '-' : '+'}</span><span class="ico ${ico}">${esc(opts.letter || '')}</span><span class="label">${esc(label)}</span>${meta || ''}</div>` +
    (open ? opts.children() : '');
}

function renderTree() {
  const tree = $('#tree');
  if (!objects) { tree.innerHTML = `<div class="empty">${conn ? 'Loading...' : 'Not connected.'}</div>`; return; }
  const o = objects;
  let html = '';
  html += node('tables', 0, 'folder', 'Tables', `<span class="count">${o.tables.length}</span>`, { children: () => o.tables.map((t, i) => {
    const prot = (t.columns || []).filter((c) => c.match_domain).length;
    const count = t.rows != null ? `<span class="meta" title="Rows">${Number(t.rows).toLocaleString()}</span>` : '';
    const temp = t.temporary ? '<span class="tag plain" title="TEMPORARY: kept in memory only">temp</span>' : '';
    return node('t:' + t.name, 1, 'table', t.name, count + temp + (prot ? `<span class="tag prot" title="${prot} PROTECTED column${prot === 1 ? '' : 's'}">${prot} protected</span>` : ''), { kind: 'table', i, letter: 'T', children: () => (t.columns || []).map((c, j) =>
      node(`c:${t.name}.${c.column}`, 2, c.match_domain ? 'pcolumn' : 'column', c.column, `<span class="meta">${esc(c.type)}</span>${c.match_domain ? `<span class="tag prot" title="PROTECTED MATCH DOMAIN ${esc(c.match_domain)}">${esc(c.match_domain)}</span>` : ''}${c.primary_key ? '<span class="tag plain" title="Primary key">pk</span>' : c.unique ? '<span class="tag plain" title="Unique">unique</span>' : ''}${c.references ? `<span class="tag plain" title="References ${esc(c.references)}">ref</span>` : ''}`, { kind: 'column', i: i + ':' + j, letter: c.match_domain ? 'P' : 'C' })).join('') });
  }).join('') });
  html += node('functions', 0, 'folder', 'Functions', `<span class="count">${o.functions.length}</span>`, { children: () => o.functions.map((f, i) =>
    node('f:' + f.function, 1, 'function', f.function, `${f.discloses ? `<span class="tag disc" title="DISCLOSES ${esc(f.discloses)}">D</span>` : ''}${f.links ? `<span class="tag link" title="LINKS ${esc(f.links)}">L</span>` : ''}`, { kind: 'function', i, letter: 'F' })).join('') });
  html += node('roles', 0, 'folder', 'Roles', `<span class="count">${o.roles.length}</span>`, { children: () => o.roles.map((r, i) =>
    node('r:' + r.role, 1, 'role', r.role, `${r.superuser ? '<span class="tag plain">superuser</span>' : ''}`, { kind: 'role', i, letter: 'R' })).join('') });
  html += node('endpoints', 0, 'folder', 'Endpoints', `<span class="count">${o.endpoints.length}</span>`, { children: () => o.endpoints.map((e, i) =>
    node('e:' + e.endpoint, 1, 'endpoint', e.endpoint, `<span class="tag ${e.path === 'MASKED_READ' ? 'masked' : 'oper'}">${esc(e.path)}</span>`, { kind: 'endpoint', i, letter: 'E' })).join('') });
  html += node('grants', 0, 'folder', 'Grants', `<span class="count">${o.grants.length}</span>`, { children: () => o.grants.map((g, i) =>
    node('g:' + i, 1, 'grant', g.grantee, `<span class="meta" title="${esc(g.privilege)}">${esc(g.privilege)}</span>`, { kind: 'grant', i, letter: 'G' })).join('') });
  tree.innerHTML = html;
}

function toggleNode(id) {
  if (openNodes.has(id)) openNodes.delete(id); else openNodes.add(id);
  store.set('openNodes', Array.from(openNodes));
  renderTree();
}

function treeEvents() {
  const tree = $('#tree');
  tree.addEventListener('click', (ev) => {
    const n = ev.target.closest('.node'); if (!n) return;
    if (ev.target.classList.contains('tw') && n.hasAttribute('aria-expanded')) { toggleNode(n.dataset.id); return; }
    selectedNode = n.dataset.id;
    $$('.node.selected', tree).forEach((x) => x.classList.remove('selected'));
    n.classList.add('selected');
    renderProps(n.dataset.kind, n.dataset.i);
  });
  tree.addEventListener('dblclick', (ev) => {
    const n = ev.target.closest('.node'); if (!n) return;
    if (n.dataset.kind === 'table') { insertText(selectFor(objects.tables[Number(n.dataset.i)])); return; }
    if (n.dataset.kind === 'function') { insertText(callFor(objects.functions[Number(n.dataset.i)])); return; }
    if (n.hasAttribute('aria-expanded')) toggleNode(n.dataset.id);
  });
  tree.addEventListener('contextmenu', (ev) => {
    const n = ev.target.closest('.node'); if (!n) return;
    ev.preventDefault();
    selectedNode = n.dataset.id;
    $$('.node.selected', tree).forEach((x) => x.classList.remove('selected'));
    n.classList.add('selected');
    renderProps(n.dataset.kind, n.dataset.i);
    const k = n.dataset.kind; const i = n.dataset.i;
    const items = [];
    if (k === 'table') {
      const t = objects.tables[Number(i)];
      items.push(['Query Data', () => { insertText(selectFor(t)); }], ['Query Data and Execute', () => runSql(selectFor(t))], ['Describe', () => runSql(`SHOW COLUMNS FROM ${t.name};`)], ['Indexes', () => runSql(`SHOW INDEXES FROM ${t.name};`, { title: 'Indexes' })], ['Stored Form', () => runSql(`SHOW STORAGE ${t.name};`)], null, ['Copy Name', () => copy(t.name)]);
    } else if (k === 'function') {
      const f = objects.functions[Number(i)];
      items.push(['Call', () => insertText(callFor(f))], ['View Source', () => insertText(`-- ${f.function}\n${f.body};\n`)], null, ['Copy Name', () => copy(f.function.replace(/\(.*/, ''))]);
    } else if (k === 'column') {
      const [ti, ci] = i.split(':').map(Number); const t = objects.tables[ti]; const c = t.columns[ci];
      items.push(['Copy Name', () => copy(c.column)], ['Copy Qualified Name', () => copy(`${t.name}.${c.column}`)]);
    } else if (k === 'role') {
      const r = objects.roles[Number(i)];
      items.push(['Privacy Matrix', () => showMatrix(r.role)], ['Copy Name', () => copy(r.role)]);
    } else {
      items.push(['Refresh', refreshObjects]);
    }
    showContextMenu(ev.clientX, ev.clientY, items);
  });
}

function selectFor(t) { return `SELECT ${(t.columns || []).map((c) => c.column).join(', ')}\nFROM ${t.name};\n`; }
function callFor(f) {
  const sig = f.function.replace(/\((.*)\)/, (m, a) => '(' + a.split(',').filter((x) => x.trim()).map((x) => x.trim().toUpperCase() === 'DATE' ? "DATE '2026-10-01'" : /INT|NUMERIC|DOUBLE|REAL/i.test(x) ? '0' : "''").join(', ') + ')');
  return `SELECT *\nFROM ${sig};\n`;
}
function copy(text) { try { navigator.clipboard.writeText(text); } catch (e) { /* clipboard unavailable */ } }

function renderProps(kind, i) {
  const p = $('#props');
  if (!objects || !kind) { p.innerHTML = ''; return; }
  const rows = (pairs) => '<table>' + pairs.filter(Boolean).map(([k, v]) => `<tr><th>${esc(k)}</th><td>${v}</td></tr>`).join('') + '</table>';
  if (kind === 'table') {
    const t = objects.tables[Number(i)];
    const prot = t.columns.filter((c) => c.match_domain);
    p.innerHTML = `<div class="ptitle">Table ${esc(t.name)}</div>` + rows([['Columns', t.columns.length], ['Protected', prot.length ? prot.map((c) => `${esc(c.column)} <span class="tag prot">${esc(c.match_domain)}</span>`).join('<br>') : 'none'], ['References', t.columns.filter((c) => c.references).map((c) => `${esc(c.column)} &rarr; ${esc(c.references)}`).join('<br>') || 'none']]);
  } else if (kind === 'column') {
    const [ti, ci] = i.split(':').map(Number); const t = objects.tables[ti]; const c = t.columns[ci];
    p.innerHTML = `<div class="ptitle">Column ${esc(t.name)}.${esc(c.column)}</div>` + rows([['Type', esc(c.type)], ['Protection', c.match_domain ? `PROTECTED MATCH DOMAIN ${esc(c.match_domain)}` : 'none'], ['Primary key', c.primary_key ? 'yes' : 'no'], ['References', esc(c.references || 'none')], c.match_domain ? ['Stored as', 'blind index and ciphertext'] : null]);
  } else if (kind === 'function') {
    const f = objects.functions[Number(i)];
    p.innerHTML = `<div class="ptitle">Function ${esc(f.function)}</div>` + rows([f.discloses ? ['Discloses', `<span class="tag disc">${esc(f.discloses)}</span>`] : null, f.links ? ['Links', `<span class="tag link">${esc(f.links)}</span>`] : null, ['Owner', esc(f.owner)], ['On MASKED_READ', esc(f.masked_path)], ['Executable by', esc(f.executable_by || 'its owner only')], f.comment ? ['Comment', esc(f.comment)] : null]) + `<pre>${esc(f.body)}</pre>`;
  } else if (kind === 'role') {
    const r = objects.roles[Number(i)];
    const g = objects.grants.filter((x) => x.grantee === r.role);
    p.innerHTML = `<div class="ptitle">Role ${esc(r.role)}</div>` + rows([['Superuser', r.superuser ? 'yes' : 'no'], ['Login', r.login ? 'yes' : 'no'], ['Endpoints', esc(r.endpoints || 'none')], ['Privileges', g.length ? g.map((x) => esc(x.privilege)).join('<br>') : 'none']]);
  } else if (kind === 'endpoint') {
    const e = objects.endpoints[Number(i)];
    p.innerHTML = `<div class="ptitle">Endpoint ${esc(e.endpoint)}</div>` + rows([['Access path', esc(e.path)], ['Roles', objects.roles.filter((r) => (r.endpoints || '').split(', ').includes(e.endpoint)).map((r) => esc(r.role)).join(', ') || 'none']]);
  } else if (kind === 'grant') {
    const g = objects.grants[Number(i)];
    p.innerHTML = `<div class="ptitle">Grant</div>` + rows([['Grantee', esc(g.grantee)], ['Privilege', esc(g.privilege)]]);
  } else p.innerHTML = '';
}

/* ------------------------------------------------------------------ context menu */

function showContextMenu(x, y, items) {
  const m = $('#ctxmenu');
  m.innerHTML = items.map((it, i) => it ? `<button type="button" role="menuitem" data-i="${i}">${esc(it[0])}</button>` : '<hr>').join('');
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = Math.min(x, window.innerWidth - r.width - 4) + 'px';
  m.style.top = Math.min(y, window.innerHeight - r.height - 4) + 'px';
  $$('button', m).forEach((b) => { b.onclick = () => { m.hidden = true; items[Number(b.dataset.i)][1](); }; });
}
document.addEventListener('mousedown', (e) => { if (!e.target.closest('#ctxmenu')) $('#ctxmenu').hidden = true; });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') $('#ctxmenu').hidden = true; });

/* ------------------------------------------------------------------ SQL windows */

function curDoc() { return docs.find((d) => d.id === activeDoc); }
function saveDocs() {
  const d = curDoc();
  if (d && editor) d.text = editor.getValue();
  store.set('docs', docs);
  store.set('activeDoc', activeDoc);
}
function isDirty(d) { return d.text !== d.saved; }

function renderDocTabs() {
  $('#doctabs').innerHTML = docs.map((d) => `<button type="button" class="dtab" role="tab" data-id="${d.id}" aria-selected="${d.id === activeDoc}" title="${esc(d.path || d.title)}">${isDirty(d) ? '<span class="dirty">*</span>' : ''}${esc(d.title)}<span class="x" data-close="1" title="Close">&#x2715;</span></button>`).join('');
  $$('#doctabs .dtab').forEach((b) => b.addEventListener('mousedown', (ev) => {
    const id = Number(b.dataset.id);
    if (ev.target.dataset.close) { ev.preventDefault(); closeDoc(id); return; }
    switchDoc(id);
  }));
}

function switchDoc(id) {
  saveDocs();
  activeDoc = id;
  const d = curDoc();
  editor.swapDoc(d.cm || (d.cm = CodeMirror.Doc(d.text, 'text/x-depsql')));
  renderDocTabs();
  saveDocs();
  editor.focus();
}

function newDoc(text, title, path) {
  saveDocs();
  const id = Math.max(0, ...docs.map((d) => d.id)) + 1;
  const t = text || '';
  docs.push({ id, title: title || 'SQL ' + id, text: t, path: path || null, saved: path ? t : '' });
  switchDoc(id);
}

async function closeDoc(id) {
  const d = docs.find((x) => x.id === id);
  if (!d) return;
  if (id === activeDoc) d.text = editor.getValue();
  if (isDirty(d) && d.text.trim()) {
    const a = await messageBox('Close SQL Window', `Save the changes to ${esc(d.title)}?`, [{ label: 'Save', value: 'save', default: true }, { label: "Don't Save", value: 'discard' }, { label: 'Cancel', value: 'cancel' }]);
    if (a === 'cancel') return;
    if (a === 'save' && !(await saveDoc(d, false))) return;
  }
  docs = docs.filter((x) => x.id !== id);
  if (!docs.length) docs.push({ id: id + 1, title: 'SQL ' + (id + 1), text: '', path: null, saved: '' });
  if (activeDoc === id || !docs.find((x) => x.id === activeDoc)) { activeDoc = -1; switchDoc(docs[docs.length - 1].id); } else { renderDocTabs(); saveDocs(); }
}

function baseName(p) { return p.split(/[\\/]/).pop(); }

async function openFile() {
  const path = await invoke('pick_open_file');
  if (!path) return;
  try {
    const text = await invoke('read_text_file', { path });
    const existing = docs.find((d) => d.path === path);
    if (existing) { switchDoc(existing.id); return; }
    const d = curDoc();
    if (d && !d.path && !editor.getValue().trim()) { d.path = path; d.title = baseName(path); d.text = d.saved = text; editor.setValue(text); renderDocTabs(); saveDocs(); }
    else newDoc(text, baseName(path), path);
    logInfo(`Opened ${path}.`);
  } catch (e) { messageBox('Open', esc(errText(e))); }
}

async function saveDoc(d, saveAs) {
  d = d || curDoc();
  if (d.id === activeDoc) d.text = editor.getValue();
  let path = d.path;
  if (!path || saveAs) {
    path = await invoke('pick_save_file', { suggested: (d.title.endsWith('.sql') ? d.title : d.title.replace(/\s+/g, '_') + '.sql') });
    if (!path) return false;
  }
  try {
    await invoke('write_text_file', { path, content: d.text });
    d.path = path; d.title = baseName(path); d.saved = d.text;
    renderDocTabs(); saveDocs();
    logInfo(`Saved ${path}.`);
    return true;
  } catch (e) { messageBox('Save', esc(errText(e))); return false; }
}

function insertText(s) {
  const doc = editor.getDoc();
  const end = { line: doc.lastLine(), ch: doc.getLine(doc.lastLine()).length };
  const pre = doc.getValue().trim() ? '\n' : '';
  doc.replaceRange(pre + s, end);
  const from = { line: end.line + (pre ? 1 : 0), ch: 0 };
  doc.setSelection(from, { line: doc.lastLine(), ch: doc.getLine(doc.lastLine()).length });
  editor.focus();
}

/* Statement boundaries: semicolons outside strings, quoted names, comments and $$ bodies. */
function statementRanges(text) {
  const out = [];
  let start = 0, i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i], c2 = text.substr(i, 2);
    if (c2 === '--') { const e = text.indexOf('\n', i); i = e < 0 ? n : e + 1; continue; }
    if (c2 === '/*') { const e = text.indexOf('*/', i + 2); i = e < 0 ? n : e + 2; continue; }
    if (c === "'" || c === '"') { let j = i + 1; while (j < n) { if (text[j] === c) { if (text[j + 1] === c) { j += 2; continue; } break; } j++; } i = j + 1; continue; }
    if (c === '$') { const m = /^\$[A-Za-z_]*\$/.exec(text.slice(i)); if (m) { const e = text.indexOf(m[0], i + m[0].length); i = e < 0 ? n : e + m[0].length; continue; } }
    if (c === ';') { out.push([start, i + 1]); start = i + 1; }
    i++;
  }
  if (text.slice(start).trim()) out.push([start, n]);
  return out;
}

function isOnlyComments(s) { return !s.replace(/--[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '').trim(); }

function statementAtCursor() {
  const text = editor.getValue();
  const pos = editor.indexFromPos(editor.getCursor());
  const ranges = statementRanges(text).filter(([a, b]) => !isOnlyComments(text.slice(a, b)));
  if (!ranges.length) return null;
  let r = ranges.find(([a, b]) => pos >= a && pos <= b);
  if (!r) r = ranges.filter(([a]) => a <= pos).pop() || ranges[0];
  // Skip leading blank lines and comments so the highlight starts at the statement.
  let a = r[0];
  const body = text.slice(a, r[1]);
  const lead = /^(\s|--[^\n]*\n?|\/\*[\s\S]*?\*\/)*/.exec(body);
  if (lead) a += lead[0].length;
  return { from: a, to: r[1], sql: text.slice(a, r[1]) };
}

function flashRange(from, to) {
  const m = editor.markText(editor.posFromIndex(from), editor.posFromIndex(to), { className: 'cm-run-line' });
  setTimeout(() => m.clear(), 500);
}

/* ------------------------------------------------------------------ running */

function now() {
  const d = new Date();
  const p = (x, k) => String(x).padStart(k || 2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function logInfo(text) {
  outputEntries.push({ time: now(), info: text });
  if (activePane === 'output') renderPane();
}

function shortSql(s) {
  const lines = s.trim().split('\n').filter((l) => !/^\s*--/.test(l));
  const first = (lines[0] || '').trim();
  return lines.length > 1 ? first + ' ...' : first;
}

function resultText(r) {
  if (r.columns && r.columns.length === 1 && r.columns[0] === 'dep_plan') return 'Plan';
  if (r.columns) return `${r.rows.length} row${r.rows.length === 1 ? '' : 's'} selected`;
  const tag = r.tag || '';
  const m = /^(INSERT \d+|UPDATE|DELETE) (\d+)$/.exec(tag);
  if (m) { const n = Number(m[2]); const verb = tag.startsWith('INSERT') ? 'inserted' : tag.startsWith('UPDATE') ? 'updated' : 'deleted'; return `${n} row${n === 1 ? '' : 's'} ${verb}`; }
  const words = { 'CREATE TABLE': 'Table created', 'CREATE FUNCTION': 'Function created', 'CREATE ROLE': 'Role created', 'CREATE ENDPOINT': 'Endpoint created', 'DROP TABLE': 'Table dropped', 'DROP FUNCTION': 'Function dropped', 'DROP ROLE': 'Role dropped', GRANT: 'Grant succeeded', REVOKE: 'Revoke succeeded', COMMIT: 'Commit complete', ROLLBACK: 'Rollback complete', BEGIN: 'Transaction started', SET: 'Set', COMMENT: 'Comment added', 'ALTER ROLE': 'Role altered' };
  return words[tag] || tag;
}

async function runSql(sql, opts) {
  opts = opts || {};
  if (!needConn()) return [];
  if (!sql || isOnlyComments(sql)) return [];
  const t0 = performance.now();
  let results;
  try {
    results = await invoke('run', { id: conn.id, sql, continueOnError: false });
  } catch (e) {
    results = [{ statement: sql, elapsedMs: 0, error: parseErr(e), notes: [] }];
  }
  const total = performance.now() - t0;
  const t = now();
  const run = ++runSeq;
  for (const r of results) outputEntries.push({ time: t, run, result: r });
  lastNotes = results.length ? results[results.length - 1].notes || [] : [];
  const errors = results.filter((r) => r.error).length;
  const notes = results.reduce((a, r) => a + (r.notes || []).length, 0);
  $('#stTiming').textContent = `${results.length} statement${results.length === 1 ? '' : 's'}${errors ? `, ${errors} failed` : ''}, ${total.toFixed(0)} ms`;

  // Result tabs: one per statement that returned rows (replacing the previous run's), like a SQL window.
  panes = panes.filter((p) => p.id === 'output' || p.keep);
  const rowResults = results.filter((r) => r.columns);
  rowResults.forEach((r, i) => {
    const isPlan = r.columns.length === 1 && r.columns[0] === 'dep_plan';
    panes.push({ id: 'r' + i, title: opts.title || (isPlan ? 'Plan' : rowResults.length > 1 ? `Result ${i + 1}` : 'Result'), result: r, plan: isPlan });
  });
  for (const r of results) {
    if (r.tag === 'BEGIN') txnState = 'open';
    else if (r.tag === 'COMMIT' || r.tag === 'ROLLBACK') txnState = null;
    else if (r.error && txnState) txnState = 'failed';
  }
  setTxnUI();
  const failed = results.find((r) => r.error);
  activePane = failed || !rowResults.length ? 'output' : 'r' + (rowResults.length - 1);
  renderPanes();
  setBottomStatus(results, total, notes);
  try { conn = await invoke('connection_info', { id: conn.id }); setConnUI(); } catch (e) { /* the connection is gone */ }
  if (results.some((r) => !r.error && /^\s*(create|drop|alter|grant|revoke|comment)\b/i.test(r.statement.replace(/^(\s|--[^\n]*\n)*/, '')))) refreshObjects();
  return results;
}

function setBottomStatus(results, total, notes) {
  const last = results[results.length - 1];
  const parts = [];
  if (last) {
    if (last.error) parts.push(`<span class="err">${esc(last.error.code || 'ERROR ' + last.error.sqlstate)}</span>`);
    else parts.push(esc(resultText(last)) + ` in ${last.elapsedMs.toFixed(1)} ms`);
  }
  if (results.length > 1) parts.push(`${results.length} statements, ${total.toFixed(0)} ms`);
  if (notes) parts.push(`<span class="dep">DEP: ${notes} note${notes === 1 ? '' : 's'}</span>`);
  $('#bstatus').innerHTML = parts.map((p) => `<span>${p}</span>`).join('');
}

function execute() {
  const sel = editor.getSelection();
  runSql(sel.trim() ? sel : editor.getValue());
}

function executeCurrent() {
  const s = statementAtCursor();
  if (!s) return;
  flashRange(s.from, s.to);
  runSql(s.sql);
}

function explain() {
  const sel = editor.getSelection();
  const s = sel.trim() ? { sql: sel } : statementAtCursor();
  if (!s) return;
  const sql = s.sql.trim().replace(/;\s*$/, '');
  if (statementRanges(sql).length > 1) { logInfo('Explain Plan works on one statement: put the cursor in it, or select it.'); showPane('output'); return; }
  if (s.from != null) flashRange(s.from, s.to);
  runSql(/^\s*explain\b/i.test(sql) ? sql : 'EXPLAIN ' + sql, { title: 'Plan' });
}

async function showMatrix(role) {
  const r = await runSql('SHOW PRIVACY MATRIX;', { title: 'Privacy Matrix' });
  const p = panes.find((x) => x.title === 'Privacy Matrix');
  if (p && role) { p.filterRole = role; renderPane(); }
  return r;
}

async function showLastNotes() {
  if (!needConn()) return;
  const notes = lastNotes;
  outputEntries.push({ time: now(), info: notes.length ? 'DEP notes of the last statement:' : 'The last statement has no DEP notes: plain SQL.' , notes });
  showPane('output');
}

async function storageDialog() {
  if (!needConn() || !objects || !objects.tables.length) return;
  $('#dlgPickTitle').textContent = 'Stored Form of Table';
  $('#pkLabel').textContent = 'Table';
  $('#pkSelect').innerHTML = objects.tables.map((t) => `<option>${esc(t.name)}</option>`).join('');
  openDialog($('#dlgPick'));
  $('#btnPick').onclick = () => { $('#dlgPick').close(); runSql(`SHOW STORAGE ${$('#pkSelect').value};`, { title: 'Stored Form' }); };
}

/* ------------------------------------------------------------------ bottom panes */

function showPane(id) { activePane = id; renderPanes(); }

function renderPanes() {
  const errs = outputEntries.filter((e) => e.result && e.result.error).length;
  $('#btabs').innerHTML = panes.map((p) => {
    let badge = '';
    if (p.id === 'output') {
      const lastRun = lastRunEntries();
      const n = lastRun.reduce((a, e) => a + ((e.result && e.result.notes) || []).length, 0);
      const e = lastRun.filter((x) => x.result && x.result.error).length;
      const when = lastRun.length ? lastRun[0].time : '';
      badge = (e ? `<span class="badge err" title="${e} statement${e === 1 ? '' : 's'} failed in the last run (${when})">${e} error${e === 1 ? '' : 's'}</span>` : '')
        + (n ? `<span class="badge dep" title="${n} DEP note${n === 1 ? '' : 's'} in the last run (${when}): how the DEP rules applied to protected values. They are listed in the Output tab.">DEP ${n}</span>` : '');
    } else if (p.result && !p.plan) badge = `<span class="count">(${p.result.rows.length} row${p.result.rows.length === 1 ? '' : 's'})</span>`;
    return `<button type="button" class="btab" role="tab" data-id="${p.id}" aria-selected="${p.id === activePane}">${esc(p.title)}${badge}</button>`;
  }).join('');
  void errs;
  $$('#btabs .btab').forEach((b) => b.addEventListener('mousedown', () => showPane(b.dataset.id)));
  renderPane();
}

// The entries of the latest run (one execution: a script, a statement, or a tool such as the
// Audit Log), told apart by run number, not by time: two runs can fall in the same second.
function lastRunEntries() {
  const last = outputEntries.filter((e) => e.result).pop();
  if (!last) return [];
  return outputEntries.filter((e) => e.result && e.run === last.run);
}

const NOTE_RE = /^(Output column (\S+) is PROTECTED|Output column (\S+) is disclosed)/;

function noteHtml(n) {
  return `<div class="line"><span class="time"></span><span class="depnote"><b>DEP:</b> ${esc(n)}</span></div>`;
}

function renderOutput() {
  if (!outputEntries.length) return '<div class="output"><div class="empty">The output of every statement appears here, with its DEP notes.</div></div>';
  const html = outputEntries.map((e) => {
    if (e.info) return `<div class="entry"><div class="line"><span class="time">${e.time}</span><span class="info">${esc(e.info)}</span></div>${(e.notes || []).map(noteHtml).join('')}</div>`;
    const r = e.result;
    let s = `<div class="entry"><div class="line"><span class="time">${e.time}</span><span class="sql"><span class="prompt">SQL&gt; </span>${esc(shortSql(r.statement))}</span></div>`;
    if (r.error) {
      const x = r.error;
      s += `<div class="line"><span class="time"></span><span class="errline">${esc(x.code ? `${x.sqlstate} ${x.code}` : `ERROR ${x.sqlstate || ''}`)}</span></div>`;
      s += `<div class="line"><span class="time"></span><span class="errmsg">${esc(x.message)}</span></div>`;
      if (x.hint) s += `<div class="line"><span class="time"></span><span class="hint">Hint: ${esc(x.hint)}</span></div>`;
      if (x.code && ERR_INFO[x.code]) s += `<div class="line"><span class="time"></span><span class="about">${esc(ERR_INFO[x.code])}</span></div>`;
    } else {
      s += `<div class="line"><span class="time"></span><span class="res">${esc(resultText(r))} in ${r.elapsedMs.toFixed(1)} ms</span></div>`;
    }
    s += (r.notes || []).map(noteHtml).join('');
    return s + '</div>';
  }).join('');
  return `<div class="output" id="outputLog">${html}</div>`;
}

function renderGrid(r, filterRole) {
  const marks = {};
  for (const n of r.notes || []) {
    const m = NOTE_RE.exec(n);
    if (m && m[2]) marks[m[2]] = 'prot';
    if (m && m[3]) marks[m[3]] = 'disc';
  }
  let rows = r.rows;
  if (filterRole) { const ri = r.columns.indexOf('role'); if (ri >= 0) rows = rows.filter((x) => x[ri] === filterRole); }
  const head = '<tr><th class="rn"></th>' + r.columns.map((c) => `<th class="${marks[c] || ''}" title="${marks[c] === 'prot' ? 'PROTECTED: returned as NULL' : marks[c] === 'disc' ? 'Disclosed by a declared function' : ''}">${esc(c)}${marks[c] ? `<span class="mk">${marks[c] === 'prot' ? 'protected' : 'disclosed'}</span>` : ''}</th>`).join('') + '</tr>';
  const body = rows.slice(0, 10000).map((row, i) => '<tr><td class="rn">' + (i + 1) + '</td>' + row.map((v) => v === null ? '<td><span class="null">(null)</span></td>' : `<td title="${esc(v)}">${esc(v)}</td>`).join('') + '</tr>').join('');
  return `<table class="grid"><thead>${head}</thead><tbody>${body}</tbody></table>${rows.length > 10000 ? `<div class="blank">Showing 10000 of ${rows.length} rows.</div>` : ''}`;
}

function renderPane() {
  const body = $('#bbody');
  const p = panes.find((x) => x.id === activePane) || panes[0];
  if (p.id === 'output') {
    body.innerHTML = renderOutput();
    body.scrollTop = body.scrollHeight;
    return;
  }
  const r = p.result;
  if (p.plan) { body.innerHTML = `<pre class="plan">${esc(r.rows.map((x) => x[0]).join('\n'))}</pre>`; return; }
  body.innerHTML = renderGrid(r, p.filterRole) + (r.rows.length ? '' : '<div class="blank">No rows selected.</div>');
  body.scrollTop = 0;
}

/* ------------------------------------------------------------------ dialogs: about, reference */

async function about(scrollToNotice) {
  const i = await invoke('app_info');
  $('#aboutFacts').innerHTML = [['Product', `${i.product} ${i.version}`], ['Engine', i.engine], ['Concept', 'Database-Enforced Privacy (DEP)'], ['Copyright', i.copyright], ['Web', 'lp.reuven.click'], ['Platform', i.platform]].map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('');
  $('#aboutLicense').textContent = i.license;
  $('#aboutNotice').textContent = i.notice;
  const third = $('#aboutThird');
  third.open = false;
  $('#aboutThirdText').textContent = '';
  third.ontoggle = async () => {
    if (third.open && !$('#aboutThirdText').textContent) $('#aboutThirdText').textContent = await invoke('third_party_notices');
  };
  $('#dlgAboutTitle').textContent = scrollToNotice ? 'Licenses and Notices' : 'About Hoopeo Studio';
  openDialog($('#dlgAbout'));
  if (scrollToNotice) $('#aboutLicenseHead').scrollIntoView();
}

function reference(toErrors) {
  $('#refBody').innerHTML = window.DEP_REFERENCE ? window.DEP_REFERENCE.html : '';
  openDialog($('#dlgRef'));
  $('#refBody').scrollTop = 0;
  if (toErrors) { const e = $('#ref-errors'); if (e) $('#refBody').scrollTop = e.offsetTop - $('#refBody').offsetTop; }
}

async function connectionInfo() {
  if (!needConn()) return;
  conn = await invoke('connection_info', { id: conn.id });
  setConnUI();
  const rows = [['Connection', conn.name], ['Role', conn.role], ['Endpoint', conn.endpoint], ['Access path', conn.path], ['Location', conn.location], conn.kind === 'server' ? ['TLS', conn.tls] : ['Storage', conn.persistent ? 'data directory' : 'in memory'], conn.serving ? ['Served on', conn.serving] : null].filter(Boolean);
  messageBox('Connection Information', '<pre>' + rows.map(([k, v]) => `${k.padEnd(12)} ${esc(v)}`).join('\n') + '</pre>');
}

/* ------------------------------------------------------------------ editing helpers */

function toggleComment() {
  const from = editor.getCursor('from').line, to = editor.getCursor('to').line;
  const lines = []; for (let l = from; l <= to; l++) lines.push(editor.getLine(l));
  const all = lines.filter((x) => x.trim()).every((x) => /^\s*--/.test(x));
  editor.operation(() => {
    for (let l = from; l <= to; l++) {
      const t = editor.getLine(l);
      if (all) { const m = /^(\s*)-- ?/.exec(t); if (m) editor.replaceRange(m[1], { line: l, ch: 0 }, { line: l, ch: m[0].length }); }
      else if (t.trim()) editor.replaceRange('-- ', { line: l, ch: 0 });
    }
  });
}
function changeCase(up) { const s = editor.getSelection(); if (s) editor.replaceSelection(up ? s.toUpperCase() : s.toLowerCase(), 'around'); }

function applyZoom() {
  document.documentElement.style.setProperty('--fs', (12 * fontScale).toFixed(1) + 'px');
  document.documentElement.style.setProperty('--code-fs', (13 * fontScale).toFixed(1) + 'px');
  store.set('fontScale', fontScale);
  if (editor) editor.refresh();
}

/* ------------------------------------------------------------------ menu */

const MENU = {
  'file.new': () => newDoc(),
  'file.open': openFile,
  'file.save': () => saveDoc(null, false),
  'file.saveAs': () => saveDoc(null, true),
  'file.close': () => closeDoc(activeDoc),
  'file.exit': async () => { saveDocs(); await invoke('quit'); },
  'edit.undo': () => editor.undo(),
  'edit.redo': () => editor.redo(),
  'edit.comment': toggleComment,
  'edit.upper': () => changeCase(true),
  'edit.lower': () => changeCase(false),
  'view.browser': () => { $('#main').classList.toggle('no-browser'); editor.refresh(); },
  'view.output': () => showPane('output'),
  'view.refresh': refreshObjects,
  'view.zoomIn': () => { fontScale = Math.min(1.6, fontScale + 0.1); applyZoom(); },
  'view.zoomOut': () => { fontScale = Math.max(0.8, fontScale - 0.1); applyZoom(); },
  'view.zoomReset': () => { fontScale = 1; applyZoom(); },
  'session.connect': () => openConnect('server'),
  'session.openLocal': () => openConnect('local'),
  'session.newLocal': openNewLocal,
  'session.serve': openServe,
  'session.stopServe': stopServing,
  'session.begin': () => runSql('BEGIN;'),
  'session.commit': () => runSql('COMMIT;'),
  'session.rollback': () => runSql('ROLLBACK;'),
  'session.info': connectionInfo,
  'session.disconnect': disconnect,
  'query.execute': execute,
  'query.executeCurrent': executeCurrent,
  'query.explain': explain,
  'query.notes': showLastNotes,
  'query.clearOutput': () => { outputEntries = []; renderPanes(); },
  'tools.matrix': () => showMatrix(null),
  'tools.grants': () => runSql('SHOW GRANTS;', { title: 'Grants' }),
  'tools.roles': () => runSql('SHOW ROLES;', { title: 'Roles' }),
  'tools.endpoints': () => runSql('SHOW ENDPOINTS;', { title: 'Endpoints' }),
  'tools.audit': () => runSql('SHOW AUDIT 500;', { title: 'Audit Log' }),
  'tools.indexes': () => runSql('SHOW INDEXES;', { title: 'Indexes' }),
  'tools.storage': storageDialog,
  'window.next': () => { const i = docs.findIndex((d) => d.id === activeDoc); switchDoc(docs[(i + 1) % docs.length].id); },
  'window.prev': () => { const i = docs.findIndex((d) => d.id === activeDoc); switchDoc(docs[(i - 1 + docs.length) % docs.length].id); },
  'help.reference': () => reference(false),
  'help.errors': () => reference(true),
  'help.licenses': () => about(true),
  'help.about': () => about(false)
};

function onMenu(id) {
  // While a dialog is open, only the dialog takes input.
  if (document.querySelector('dialog[open]') && !/^(edit\.|view\.zoom|file\.exit)/.test(id)) return;
  const f = MENU[id];
  if (f) Promise.resolve(f()).catch((e) => logInfo(errText(e)));
}

/* ------------------------------------------------------------------ layout */

function splitters() {
  const main = $('#main'), work = $('.work');
  const bw = store.get('browserW', null); if (bw) main.style.setProperty('--browser-w', bw + 'px');
  const bh = store.get('bottomH', null); if (bh) work.style.setProperty('--bottom-h', bh + 'px');
  const drag = (el, onMove, onEnd) => el.addEventListener('mousedown', (e) => {
    e.preventDefault();
    const move = (ev) => { onMove(ev); editor.refresh(); };
    const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); onEnd(); };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', up);
  });
  drag($('#vsplit'), (ev) => main.style.setProperty('--browser-w', Math.min(560, Math.max(180, ev.clientX)) + 'px'), () => store.set('browserW', parseInt(getComputedStyle(main).getPropertyValue('--browser-w'), 10)));
  drag($('#hsplit'), (ev) => { const r = work.getBoundingClientRect(); work.style.setProperty('--bottom-h', Math.min(r.height - 140, Math.max(90, r.bottom - ev.clientY)) + 'px'); }, () => store.set('bottomH', parseInt(getComputedStyle(work).getPropertyValue('--bottom-h'), 10)));
}

/* ------------------------------------------------------------------ self test */

// HOOPEO_STUDIO_SELFTEST: plain SQL (run as admin on an empty database in memory), or JSON
// {"steps": [{"connect": profile, "password": "..."}, {"sql": "..."}, {"menu": "query.explain"}, {"wait": 300}]}.
async function selfTest(script) {
  let steps;
  try { steps = JSON.parse(script).steps; } catch (e) { steps = [{ connect: { name: 'Self test', kind: 'memory', user: 'admin', endpoint: 'operational' } }, { sql: script }, { menu: 'query.execute' }]; }
  for (const s of steps) {
    try {
      if (s.connect) await connectProfile(Object.assign({ host: '', port: 5433, dataDir: '', tls: 'disable', caFile: '', servePort: null }, s.connect), s.password, s.adminPassword);
      if (s.sql != null) { editor.setValue(s.sql); if (s.cursor != null) editor.setCursor(editor.posFromIndex(s.cursor)); }
      if (s.menu) await Promise.resolve(MENU[s.menu]());
      if (s.select) { const n = $(`.node[data-id="${s.select}"]`); if (n) n.click(); }
      if (s.open) { openNodes.add(s.open); renderTree(); }
      if (s.pane) showPane(s.pane);
      if (s.wait) await new Promise((r) => setTimeout(r, s.wait));
      if (s.log) console.log('[selftest]', JSON.stringify({ conn, panes: panes.map((p) => p.title), output: outputEntries.map((e) => e.info || [e.result.statement, e.result.error ? e.result.error.code || e.result.error.sqlstate : resultText(e.result), e.result.notes]) }));
    } catch (e) {
      logInfo('Self test: ' + errText(e));
    }
  }
}

/* ------------------------------------------------------------------ boot */

function boot() {
  applyZoom();
  const d = curDoc() || docs[0];
  activeDoc = d.id;
  d.cm = CodeMirror.Doc(d.text, 'text/x-depsql');
  editor = CodeMirror($('#editorHost'), {
    value: d.cm,
    mode: 'text/x-depsql',
    lineNumbers: true,
    indentUnit: 4,
    tabSize: 4,
    indentWithTabs: false,
    matchBrackets: true,
    styleActiveLine: true,
    extraKeys: { Tab: (cm) => cm.replaceSelection('    ') }
  });
  editor.on('change', () => { const c = curDoc(); if (c) c.text = editor.getValue(); clearTimeout(boot.t); boot.t = setTimeout(() => { saveDocs(); renderDocTabs(); }, 300); });
  editor.on('cursorActivity', () => { const c = editor.getCursor(); $('#stCursor').textContent = `Ln ${c.line + 1}, Col ${c.ch + 1}`; });
  renderDocTabs();
  renderTree();
  renderPanes();
  setConnUI();
  splitters();
  treeEvents();

  $('#pfKind').onchange = kindChanged;
  $('#pfTls').onchange = kindChanged;
  $('#btnDoConnect').onclick = doConnect;
  $('#btnTestProfile').onclick = testConnection;
  $('#btnSaveProfile').onclick = async () => { try { const p = formProfile(); await invoke('save_profile', { profile: p }); profiles = await invoke('list_profiles'); await selectProfile(profiles.find((x) => x.name === p.name) || p); $('#pfOk').textContent = 'Saved. The password is not stored.'; $('#pfOk').hidden = false; } catch (e) { formError(e); } };
  $('#btnDeleteProfile').onclick = async () => { if (!selectedProfile) return; await invoke('delete_profile', { name: selectedProfile.name }); profiles = await invoke('list_profiles'); await selectProfile(profiles[0]); };
  $('#pfPassword').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); doConnect(); } });
  $('#pfDataDirPick').onclick = async () => { const p = await invoke('pick_folder'); if (p) $('#pfDataDir').value = p; };
  $('#pfCaPick').onclick = async () => { const p = await invoke('pick_any_file'); if (p) $('#pfCaFile').value = p; };
  $('#btnCreateLocal').onclick = createLocal;
  $('#nlDirPick').onclick = async () => { const p = await invoke('pick_folder'); if (p) $('#nlDir').value = p; };
  $('#nlName').addEventListener('input', async () => { $('#nlDir').value = await invoke('suggest_data_dir', { name: $('#nlName').value }); });
  $('#btnServe').onclick = doServe;
  window.addEventListener('beforeunload', saveDocs);

  if (T.event) T.event.listen('menu', (e) => onMenu(e.payload));

  invoke('app_ready');
  invoke('self_test').then((script) => {
    if (script) { selfTest(script); return; }
    openConnect();
  });
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot); else boot();
})();
