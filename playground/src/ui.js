(function () {
'use strict';
const $ = (s, r) => (r || document).querySelector(s);
const $$ = (s, r) => Array.from((r || document).querySelectorAll(s));
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const store = { get(k, d) { try { const v = localStorage.getItem('depsql5.' + k); return v === null ? d : v; } catch (e) { return d; } }, set(k, v) { try { localStorage.setItem('depsql5.' + k, v); } catch (e) { /* unavailable */ } } };

/* Each example runs on the three panes. `sqlite` is the regular SQL version when it differs;
   null when regular SQL has no equivalent. */
const EX = [
  { label: 'Select', sql: 'SELECT full_name, national_id, health_fund\nFROM patients;' },
  { label: 'Lookup by ID', sql: "SELECT full_name, national_id, health_fund\nFROM patients\nWHERE national_id = '123456782';" },
  { label: 'Partial match', sql: "SELECT full_name\nFROM patients\nWHERE national_id LIKE '1234%';" },
  { label: 'List of IDs', sql: "SELECT full_name, health_fund\nFROM patients\nWHERE national_id IN ('123456782', '345678904');" },
  { label: 'Join one', sql: "SELECT p.full_name, m.diagnosis, m.recorded_at\nFROM patients AS p\nJOIN medical_records AS m\n  ON m.patient_national_id = p.national_id\nWHERE p.national_id = '123456782';" },
  { label: 'Join all', sql: 'SELECT p.full_name, m.diagnosis\nFROM patients AS p\nJOIN medical_records AS m\n  ON m.patient_national_id = p.national_id;' },
  { label: 'Linking', sql: 'SELECT *\nFROM diagnoses_per_fund();', sqlite: 'SELECT *\nFROM diagnoses_per_fund;' },
  { label: 'Disclosing', sql: 'SELECT *\nFROM monthly_fund_report(current_date);', sqlite: "SELECT *\nFROM monthly_fund_report\nWHERE strftime('%Y-%m', visit_date) = strftime('%Y-%m', 'now', 'localtime');" },
  { label: 'Join disclosed', sql: 'SELECT r.national_id, m.diagnosis\nFROM monthly_fund_report(current_date) AS r\nJOIN medical_records AS m\n  ON m.patient_national_id = r.national_id;', sqlite: "SELECT r.national_id, m.diagnosis\nFROM monthly_fund_report AS r\nJOIN medical_records AS m\n  ON m.patient_national_id = r.national_id\nWHERE strftime('%Y-%m', r.visit_date) = strftime('%Y-%m', 'now', 'localtime');" },
  { label: 'Update', sql: "UPDATE patients\nSET health_fund = 'Clalit'\nWHERE national_id = '123456782';" },
  { label: 'Transaction', sql: "-- Changes are kept only at COMMIT; ROLLBACK TO undoes part of a transaction.\nBEGIN;\nUPDATE patients SET health_fund = 'Maccabi' WHERE national_id = '123456782';\nSAVEPOINT before_phone;\nUPDATE patients SET phone = '+972-50-9999999' WHERE patient_id = 1;\nROLLBACK TO SAVEPOINT before_phone;\nSELECT full_name, health_fund FROM patients WHERE patient_id = 1;\nROLLBACK;" },
  { label: 'Constraints', sql: "-- A duplicate ordinary key is shown in the error; a duplicate PROTECTED value is not.\nINSERT INTO patients (patient_id, full_name) VALUES (1, 'Second Dana');\nINSERT INTO patients (patient_id, national_id, full_name) VALUES (7, '123456782', 'Dana Twin');\nINSERT INTO medical_records (record_id, patient_national_id, diagnosis) VALUES (207, '999999999', 'Flu');" },
  { label: 'Indexes', sql: "CREATE INDEX ON appointments (visit_type);\nEXPLAIN SELECT appointment_id FROM appointments WHERE visit_type = 'Vaccination';\nEXPLAIN SELECT full_name FROM patients WHERE national_id = '123456782';\nSHOW INDEXES FROM patients;\nDROP INDEX appointments_visit_type_idx;", sqlite: "CREATE INDEX appointments_visit_type_idx ON appointments (visit_type);\nEXPLAIN QUERY PLAN SELECT appointment_id FROM appointments WHERE visit_type = 'Vaccination';\nEXPLAIN QUERY PLAN SELECT full_name FROM patients WHERE national_id = '123456782';\nDROP INDEX appointments_visit_type_idx;" },
  { label: 'Roles', sql: "-- The reception application may match national_id, but not phone.\nSET ROLE reception_app;\nSELECT full_name, phone FROM patients WHERE national_id = '123456782';\nSELECT full_name FROM patients WHERE national_id = '123456782';\nRESET ROLE;", sqlite: "SELECT full_name, phone FROM patients WHERE national_id = '123456782';\nSELECT full_name FROM patients WHERE national_id = '123456782';" },
  { label: 'Match limit', sql: "ALTER ROLE reception_app MATCH LIMIT 2 PER MINUTE;\nSET ROLE reception_app;\nSELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893', '345678904');\nRESET ROLE;\nALTER ROLE reception_app MATCH LIMIT DEFAULT;", sqlite: "SELECT full_name FROM patients WHERE national_id IN ('123456782', '234567893', '345678904');" },
  { label: 'Audit', sql: 'SHOW AUDIT 15;', sqlite: null },
  { label: 'Who can match', sql: 'SHOW PRIVACY MATRIX;', sqlite: null },
  { label: 'Read IDs', sql: 'SELECT national_id\nFROM patients;' }
];
const TOUR = [
  { index: 1, label: '1. Lookup', title: 'Find one patient. Keep the ID protected.', description: 'Regular SQL returns the ID. DEP operational finds the same patient and returns NULL for the ID. DEP masked read refuses the lookup entirely.' },
  { index: EX.findIndex(example => example.label === 'Read IDs'), label: '2. Read IDs', title: 'A match privilege does not export every identifier.', description: 'Regular SQL returns all six IDs. Both DEP paths return six NULLs. The rows still exist; their protected values cannot leave through an ordinary SELECT. Pattern searches such as LIKE are a different operation, and are refused.' },
  { index: 4, label: '3. Join one', title: 'Connect the records of the patient in front of you.', description: 'The WHERE equality anchors this protected join to one supplied value. DEP operational permits it; masked read cannot match protected inputs.' },
  { index: 5, label: '4. Join everyone', title: 'Whole-dataset linkage needs an explicit exception.', description: 'Removing the anchor changes the scope of the operation. DEP operational refuses the protected join with ZD107. Approved bulk linkage belongs in a LINKS function.' },
  { index: 7, label: '5. Disclose', title: 'Release an identifier through a declared function.', description: 'The demo admin can call this approved DISCLOSES function on the operational path. Masked read cannot disclose. In an application, grant EXECUTE only to the role that needs it.' }
];
let tourStep = 0;
function updateTour(i) {
  const at = TOUR.findIndex((step) => step.index === Number(i));
  if (at >= 0) { tourStep = at; $('#tour-title').textContent = TOUR[at].title; $('#tour-description').textContent = TOUR[at].description; }
  else { $('#tour-title').textContent = 'Explore another example'; $('#tour-description').textContent = 'These examples share a synthetic database. Reset restores the demo; use the five tour steps to return to the guided path.'; }
  $$('#tour-steps button').forEach((button, index) => { if (index === at) button.setAttribute('aria-current', 'step'); else button.removeAttribute('aria-current'); });
  $('#tour-next').textContent = at === TOUR.length - 1 ? 'Restart tour ↺' : 'Next example →';
  const picker = $('#example-picker'); if (picker) picker.value = String(i);
}
const MINE = "SELECT full_name, health_fund\nFROM patients\nWHERE national_id = '234567893';";

let SQLlib, server, sqlite, current = null;
const conn = {};

function start() {
  /* depsql.js: one DEP server with two connections. sql.js: one ordinary SQLite database. */
  if (conn.masked) conn.masked.close();
  if (conn.op) conn.op.close();
  server = new window.DepSQL.Server(SQLlib, { setup: window.DepClinic.clinicSetup });
  conn.masked = server.connect('admin', 'analytics');
  conn.op = server.connect('admin', 'operational');
  if (sqlite) sqlite.close();
  sqlite = new SQLlib.Database();
  sqlite.exec('PRAGMA foreign_keys = ON');
  sqlite.exec(window.DepClinic.clinicSqlite());
}

/* ---------- rendering */
function cell(v, tag) {
  if (v === null || v === undefined) return `<span class="null${tag === 'protected' ? ' prot' : ''}">NULL</span>`;
  return esc(v);
}
function table(cols, rows) {
  const head = cols.map((c) => `<th>${esc(c.name)}${c.tag ? `<span class="chip ${c.tag === 'protected' ? 'prot' : 'disc'}">${c.tag}</span>` : ''}</th>`).join('');
  const body = rows.map((r) => '<tr>' + r.map((v, i) => `<td>${cell(v, cols[i] && cols[i].tag)}</td>`).join('') + '</tr>').join('');
  return `<div class="tablewrap"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
}
function notesHtml(r) { return (r.notes || []).length ? `<details class="notes"><summary>Why DEP handled it this way (${r.notes.length})</summary>${r.notes.map((n) => `<div><b>NOTE:</b>  ${esc(n)}</div>`).join('')}</details>` : ''; }
function resultHtml(r) {
  const stmt = r.text ? `<div class="stmt">${esc(r.text.replace(/\s+/g, ' ').slice(0, 140))}</div>` : '';
  if (!r.ok) {
    const e = r.error;
    const lines = [`<b>ERROR:</b>  ${esc(e.message)}`];
    if (e.hint) lines.push(`<b>HINT:</b>  ${esc(e.hint)}`);
    if (e.sqlstate) lines.push(`<b>SQLSTATE:</b>  ${esc(e.sqlstate)}${e.code ? ' (' + esc(e.code) + ')' : ''}`);
    return `<div class="res">${stmt}${notesHtml(r)}<pre class="err">${lines.join('\n')}</pre></div>`;
  }
  if (r.kind === 'rows') return `<div class="res">${stmt}${notesHtml(r)}${table(r.columns, r.rows)}<div class="cmd muted">(${r.rows.length} row${r.rows.length === 1 ? '' : 's'})</div></div>`;
  return `<div class="res">${stmt}${notesHtml(r)}<div class="cmd">${esc(r.tag)}</div></div>`;
}
function summary(list) {
  const failed = list.find((r) => !r.ok);
  if (failed) return { text: failed.error.code || 'ERROR' + (failed.error.sqlstate ? ' ' + failed.error.sqlstate : ''), cls: 'err' };
  const last = list[list.length - 1];
  if (!last) return { text: '', cls: '' };
  if (last.kind === 'rows') return { text: `${last.rows.length} row${last.rows.length === 1 ? '' : 's'}`, cls: 'ok' };
  return { text: String(last.tag).split(' (')[0], cls: 'ok' };
}
function renderPane(id, list, note) {
  const showStmt = list.length > 1;
  $('#out-' + id).innerHTML = (note && (!showStmt || !list.length) ? `<pre class="ran">${esc(note)}</pre>` : '') + (list.length ? list.map((r) => resultHtml(showStmt ? r : Object.assign({}, r, { text: '' }))).join('') : '');
  const s = summary(list); const el = $('#sum-' + id); el.textContent = s.text; el.className = 'sum ' + s.cls;
}
function paneStatus() {
  for (const p of ['masked', 'op']) {
    const c = conn[p];
    $('#who-' + p).textContent = `${c.role}@clinic:${c.endpoint}`;
    const t = $('#txn-' + p);
    t.hidden = !c.inTransaction;
    t.textContent = c.transactionFailed ? 'transaction failed' : 'in transaction';
    t.className = 'txn' + (c.transactionFailed ? ' bad' : '');
  }
}

/* ---------- running */
/* Regular SQL: straight to sql.js, statement by statement, like a SQLite shell. */
function runSqlite(src) {
  const out = [];
  try {
    for (const st of sqlite.iterateStatements(src)) {
      const text = st.getSQL();
      try {
        const cols = st.getColumnNames();
        if (cols.length) {
          const rows = []; while (st.step()) rows.push(st.get());
          out.push({ ok: true, text, kind: 'rows', columns: cols.map((n) => ({ name: n })), rows });
        } else {
          st.step();
          const verb = (text.trim().match(/^\w+/) || [''])[0].toUpperCase();
          const n = sqlite.getRowsModified();
          out.push({ ok: true, text, kind: 'cmd', tag: verb === 'INSERT' ? 'INSERT 0 ' + n : ['UPDATE', 'DELETE'].includes(verb) ? verb + ' ' + n : verb });
        }
      } catch (e) { out.push({ ok: false, text, error: { message: e.message } }); }
      finally { st.free(); }
    }
  } catch (e) { out.push({ ok: false, error: { message: e.message } }); }
  return out;
}
function runAll() {
  if (!server) return;
  const src = $('#sql').value; if (!src.trim()) return;
  const ex = current !== null && current !== 'mine' ? EX[Number(current)] : null;
  const out = { masked: [], op: [] };
  let stmts;
  try { stmts = window.DepSQL.split(src); }
  catch (e) { const r = { ok: false, text: src, error: { message: e.message, sqlstate: e.sqlstate || '42601' } }; out.masked.push(r); out.op.push(r); stmts = []; }
  for (const s of stmts) {
    if (s.toks[0].t === 'meta') { const r = { ok: false, text: s.text, error: { message: 'meta-commands are not available here', sqlstate: '42601' } }; out.masked.push(r); out.op.push(r); continue; }
    for (const p of ['masked', 'op']) out[p].push(...conn[p].exec(s));
  }
  if (ex && ex.sqlite === null) renderPane('plain', [], 'No equivalent in regular SQL.');
  else {
    const sqliteSrc = ex && ex.sqlite ? ex.sqlite : src;
    renderPane('plain', runSqlite(sqliteSrc), sqliteSrc !== src ? sqliteSrc : null);
  }
  renderPane('masked', out.masked); renderPane('op', out.op);
  paneStatus();
  refreshSide();
}

/* ---------- tabs */
function renderTabs() {
  $('#extabs').innerHTML = '<label>Choose an example <select id="example-picker" aria-label="Example">' + EX.map((ex, i) => `<option value="${i}">${esc(ex.label)}</option>`).join('') + '<option value="mine">Your code</option></select></label>';
  $('#example-picker').addEventListener('change', (event) => select(event.target.value));
  $('#tour-steps').innerHTML = TOUR.map((step, index) => `<button type="button" class="tour-step" data-step="${index}">${esc(step.label)}</button>`).join('');
  $$('#tour-steps button').forEach((button) => button.addEventListener('click', () => select(String(TOUR[Number(button.dataset.step)].index))));
  $('#tour-next').addEventListener('click', () => select(String(TOUR[(tourStep + 1) % TOUR.length].index)));
  $('#tour-mode').addEventListener('click', () => {
    const open = $('#extabs').hidden;
    $('#extabs').hidden = !open;
    $('#tour-mode').setAttribute('aria-pressed', String(open));
    $('#tour-mode').textContent = open ? 'Hide example browser' : 'Browse all examples';
    if (open) $('#example-picker').focus();
  });
}
function markTab(i) { updateTour(i); }
function select(i) {
  current = i; markTab(i);
  $('#sql').value = i === 'mine' ? store.get('mine', MINE) : EX[Number(i)].sql;
  document.dispatchEvent(new CustomEvent('hoopeo:querychange'));
  runAll();
}

/* ---------- schema box (as the operational connection sees it) */
function refreshSide() {
  const d = server.describe(conn.op);
  $('#sp-schema').innerHTML = `<div class="tbls">${d.tables.map((t) => `<div class="tbl"><h3>${esc(t.name)} <span class="ty">${t.rows === null ? '' : t.rows + ' rows'}${t.temporary ? ' · temporary' : ''}</span></h3><ul>${t.columns.map((c) => `<li><code>${esc(c.name)}</code> <span class="ty">${esc(c.type)}</span>${c.domain ? `<span class="chip prot">protected · ${esc(c.domain)}</span>` : ''}${c.pk ? '<span class="chip key">pk</span>' : c.unique ? '<span class="chip key">unique</span>' : ''}${c.references ? ` <span class="ref">→ ${esc(c.references.table)}.${esc(c.references.col)}</span>` : ''}</li>`).join('')}</ul><div class="idx">${t.indexes.map((i) => `<div><code>${esc(i.name)}</code> <span class="ty">(${esc(i.columns.join(', '))})${i.primary ? ' primary key' : i.unique ? ' unique' : ''}${i.auto ? '' : ' · CREATE INDEX'}</span></div>`).join('')}</div></div>`).join('')}</div>`;
  $('#sp-catalog').innerHTML = d.functions.map((f) => `<div class="fn"><h3>${esc(f.signature)}</h3><div class="ty">${[f.discloses.length ? 'DISCLOSES ' + f.discloses.join(', ') : '', f.links.length ? 'LINKS ' + f.links.join(', ') : ''].filter(Boolean).map(esc).join(' · ') || 'not declared'} · owner ${esc(f.owner)} · executable by ${esc(f.executableBy.join(', ') || 'owner only')} · masked path ${f.maskedOk ? 'yes' : 'no'}</div>${f.comment ? `<div class="muted">${esc(f.comment)}</div>` : ''}<pre>${esc(f.body.trim())}</pre></div>`).join('') || '<p class="muted">No functions.</p>';
  $('#sp-access').innerHTML = `<h4>Roles</h4>${table([{ name: 'role' }, { name: 'endpoints' }, { name: 'match limit' }], d.roles.map((r) => [r.name + (r.superuser ? ' (superuser)' : ''), r.endpoints.join(', '), r.matchLimit === 'default' ? '' : r.matchLimit]))}<h4>Endpoints</h4>${table([{ name: 'endpoint' }, { name: 'path' }], d.endpoints.map((e) => [e.name, e.path]))}<h4>Grants</h4>${table([{ name: 'grantee' }, { name: 'privilege' }], d.grants.map((g) => [g.grantee, g.privilege]))}`;
  const recs = server.auditLog.slice(-60).reverse();
  $('#sp-audit').innerHTML = recs.length ? table([{ name: 'seq' }, { name: 'role' }, { name: 'path' }, { name: 'kind' }, { name: 'object' }, { name: 'domain' }, { name: 'count' }, { name: 'detail' }], recs.map((r) => [r.seq, r.role, r.path, r.kind, r.object, r.domain, r.count || null, r.detail])) : '<p class="muted">No audit records yet. Every protected match, every call of a declared function, and every refused statement is recorded here, without values.</p>';
  const sel = $('#storageTable'); const cur = sel.value || 'patients';
  sel.innerHTML = d.tables.map((t) => `<option${t.name === cur ? ' selected' : ''}>${esc(t.name)}</option>`).join('');
  renderStorage();
}
function renderStorage() {
  const v = server.storage($('#storageTable').value, conn.op); if (!v) { $('#storageView').innerHTML = ''; return; }
  const short = (x) => (x === null ? null : String(x).length > 22 ? String(x).slice(0, 20) + '…' : x);
  $('#storageView').innerHTML = table(v.columns.map((n) => ({ name: n })), v.rows.map((r) => r.map((x, i) => (/__ct$/.test(v.columns[i]) || /^ie1:/.test(String(x)) ? short(x) : x))));
}

async function boot() {
  try {
    const opts = window.__SQLJS_WASM__ ? { wasmBinary: Uint8Array.from(atob(window.__SQLJS_WASM__), (c) => c.charCodeAt(0)) } : { locateFile: (f) => f };
    SQLlib = await window.initSqlJs(opts);
  } catch (e) { $('#status').textContent = 'The database could not start: ' + e.message; return; }
  start();
  $('#status').textContent = '';
  $('#ver').textContent = 'DEP.js ' + window.DepSQL.version;
  renderTabs();
  $$('.stab').forEach((b) => b.addEventListener('click', () => { $$('.stab').forEach((x) => x.setAttribute('aria-selected', x === b ? 'true' : 'false')); $$('.spanel').forEach((p) => { p.hidden = p.id !== b.dataset.p; }); }));
  $('#run').addEventListener('click', runAll);
  $('#storageTable').addEventListener('change', renderStorage);
  $('#reset').addEventListener('click', () => { start(); select(current === null ? '0' : current); });
  $('#sql').addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); runAll(); }
    if (e.key === 'Tab' && e.altKey && !e.shiftKey) { e.preventDefault(); const t = e.target, s = t.selectionStart; t.value = t.value.slice(0, s) + '    ' + t.value.slice(t.selectionEnd); t.selectionStart = t.selectionEnd = s + 4; }
  });
  $('#sql').addEventListener('input', () => { if (current !== 'mine') { current = 'mine'; markTab('mine'); } store.set('mine', $('#sql').value); });
  const requested = new URLSearchParams(location.search).get('example');
  const homepageExamples = {'lookup':'1','read-all':String(EX.findIndex(example => example.label === 'Read IDs')),'join-one':'4','join-all':'5'};
  select(homepageExamples[requested] || '1');
}
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot); else boot();
})();
