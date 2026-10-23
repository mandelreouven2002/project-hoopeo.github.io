/*!
 * depsql.js 0.5.2 (DEP.js)
 * DEP SQL in the browser: a simulator of DEP Engine 0.5 that runs on sql.js (SQLite).
 *
 * DEP (Database-Enforced Privacy) is a design proposal by Reuven Mandelovich (CC BY 4.0).
 * DEP Engine is its implementation on Apache DataFusion. This file follows the engine: it parses
 * DEP SQL, applies the same rules with the same errors, notes and audit records, and rewrites
 * each admitted statement into SQLite for sql.js to run. Everything runs in the caller's process
 * with simulated cryptography, so it illustrates the rules and protects nothing.
 *
 * Licensed under the Apache License 2.0.
 *
 * Usage:
 *   const SQL = await initSqlJs();
 *   const server = new DepSQL.Server(SQL, { setup: 'CREATE TABLE ...' });
 *   const conn = server.connect('admin', 'operational');
 *   const results = conn.exec("SELECT ... WHERE national_id = $1", ['123456782']);
 *
 * Stages: tokenizer -> parser (AST) -> analyzer (DEP rules) -> rewriter (SQLite) -> sql.js
 */
(function (global) {
'use strict';

const VERSION = '0.5.2';

/* ------------------------------------------------------------------ errors (as in the engine) */

const SQLSTATE = {
  DEP_REVEAL_OUTSIDE_DISCLOSURE: 'ZD001', DEP_DECLARATION_PRIVILEGE: 'ZD002', DEP_DECLARED_LANGUAGE: 'ZD003',
  DEP_DECLARED_CALL: 'ZD004', DEP_PUBLIC_EXECUTE: 'ZD005', DEP_REFERENCE_MISMATCH: 'ZD006',
  DEP_PROTECTED_OPERATION: 'ZD101', DEP_ENDPOINT_OPERATION: 'ZD102', DEP_MATCH_DOMAIN_MISMATCH: 'ZD103',
  DEP_DISCLOSED_VALUE_USE: 'ZD104', DEP_UNQUALIFIED_ACCESS: 'ZD105', DEP_PATH_IMMUTABLE: 'ZD106',
  DEP_UNANCHORED_MATCH: 'ZD107', DEP_UNSUPPORTED_FORM: 'ZD108', DEP_MATCH_RATE_LIMITED: 'ZD109'
};

class SqlError extends Error {
  constructor(message, o) {
    super(message);
    o = o || {};
    this.code = o.code || null;                       // DEP code, or null for an ordinary SQL error
    this.sqlstate = o.sqlstate || (o.code ? SQLSTATE[o.code] || 'ZD901' : 'XX000');
    this.hint = o.hint || null;
  }
}
const dep = (code, message, hint) => new SqlError(message, { code, hint });
const sqlerr = (sqlstate, message, hint) => new SqlError(message, { sqlstate, hint });
const denied = (message, hint) => sqlerr('42501', message, hint);
const unsupported = (m) => dep('DEP_UNSUPPORTED_FORM', m, 'DEP rejects forms it cannot analyze rather than run them with uncertain protection.');
const masked = (what) => dep('DEP_ENDPOINT_OPERATION', `${what} unavailable on a MASKED_READ connection.`, 'This operation requires an OPERATIONAL connection.');
const protectedOp = (what, desc) => dep('DEP_PROTECTED_OPERATION', `${what} is not supported for PROTECTED values (${desc}).`, 'PROTECTED values support only full-value equality matching. To release a value, use a disclosing function.');
const disclosedUse = (desc, place) => dep('DEP_DISCLOSED_VALUE_USE', `${desc} is a disclosed value and cannot be used in ${place}.`, 'Disclosed values can only be returned. Perform the match on PROTECTED fields inside a declared function, before REVEAL.');
const domainMismatch = (a, b) => dep('DEP_MATCH_DOMAIN_MISMATCH', `${a.desc} (match domain ${a.domain}) cannot be compared with ${b.desc} (match domain ${b.domain}).`, 'Values from different match domains are different kinds of value.');
const superOnly = (what) => denied(`permission denied to ${what}`, 'Only a superuser can do this.');

/* ------------------------------------------------------------------ simulated cryptography */

const K = new Uint32Array([0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
  0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,
  0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,
  0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,
  0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
  0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,
  0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2]);
function sha256(bytes) {
  const l = bytes.length, total = Math.ceil((l + 9) / 64) * 64;
  const m = new Uint8Array(total); m.set(bytes); m[l] = 0x80;
  const bits = l * 8, dv = new DataView(m.buffer);
  dv.setUint32(total - 4, bits >>> 0); dv.setUint32(total - 8, Math.floor(bits / 4294967296));
  const H = new Uint32Array([0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19]);
  const W = new Uint32Array(64);
  for (let i = 0; i < total; i += 64) {
    for (let t = 0; t < 16; t++) W[t] = dv.getUint32(i + t * 4);
    for (let t = 16; t < 64; t++) {
      const a = W[t - 15], b = W[t - 2];
      const s0 = ((a >>> 7) | (a << 25)) ^ ((a >>> 18) | (a << 14)) ^ (a >>> 3);
      const s1 = ((b >>> 17) | (b << 15)) ^ ((b >>> 19) | (b << 13)) ^ (b >>> 10);
      W[t] = (W[t - 16] + s0 + W[t - 7] + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, h] = H;
    for (let t = 0; t < 64; t++) {
      const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
      const t1 = (h + S1 + ((e & f) ^ (~e & g)) + K[t] + W[t]) >>> 0;
      const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
      const t2 = (S0 + ((a & b) ^ (a & c) ^ (b & c))) >>> 0;
      h = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    H[0] += a; H[1] += b; H[2] += c; H[3] += d; H[4] += e; H[5] += f; H[6] += g; H[7] += h;
  }
  const out = new Uint8Array(32), odv = new DataView(out.buffer);
  for (let i = 0; i < 8; i++) odv.setUint32(i * 4, H[i]);
  return out;
}
const enc = (s) => new TextEncoder().encode(s);
const dec = (b) => new TextDecoder().decode(b);
function concat(...arrs) { const n = arrs.reduce((s, a) => s + a.length, 0); const o = new Uint8Array(n); let p = 0; for (const a of arrs) { o.set(a, p); p += a.length; } return o; }
function hmac(key, msg) {
  const k = key.length > 64 ? sha256(key) : key; const kk = new Uint8Array(64); kk.set(k);
  return sha256(concat(kk.map((x) => x ^ 0x5c), sha256(concat(kk.map((x) => x ^ 0x36), msg))));
}
const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const b64 = (b) => { let s = ''; b.forEach((x) => { s += String.fromCharCode(x); }); return btoa(s); };
const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
function randomBytes(n) {
  const b = new Uint8Array(n);
  if (global.crypto && global.crypto.getRandomValues) global.crypto.getRandomValues(b);
  else for (let i = 0; i < n; i++) b[i] = Math.floor(Math.random() * 256);
  return b;
}
function keystream(key, nonce, n) {
  const out = new Uint8Array(n); let ctr = 0, p = 0;
  while (p < n) { const blk = sha256(concat(key, nonce, enc(String(ctr++)))); for (let i = 0; i < blk.length && p < n; i++) out[p++] = blk[i]; }
  return out;
}

/* The keyring of one server: keys derived per purpose and match domain from a master key.
   Index entries have the engine's form (ie1: + 16 bytes of HMAC-SHA256, hex) and ciphertexts
   its framing (ct1: + base64(24-byte nonce || ciphertext || tag)); the cipher is a stand-in. */
class Keyring {
  constructor(master) { this.master = master || randomBytes(32); this.cache = new Map(); }
  derive(purpose, domain) {
    const k = purpose + '|' + domain;
    if (!this.cache.has(k)) this.cache.set(k, hmac(this.master, enc('dep ' + k)));
    return this.cache.get(k);
  }
  fingerprint() { return hex(sha256(this.master)).slice(0, 16); }
  indexEntry(domain, v) { return 'ie1:' + hex(hmac(this.derive('index', domain), enc(v)).slice(0, 16)); }
  encrypt(domain, v) {
    const k = this.derive('aead', domain), nonce = randomBytes(24), pt = enc(v), ks = keystream(k, nonce, pt.length);
    const ct = pt.map((x, i) => x ^ ks[i]); const tag = hmac(k, concat(nonce, ct, enc(domain))).slice(0, 16);
    return 'ct1:' + b64(concat(nonce, ct, tag));
  }
  decrypt(domain, s) {
    if (typeof s !== 'string' || !s.startsWith('ct1:')) return null;
    let raw; try { raw = unb64(s.slice(4)); } catch (e) { return null; }
    if (raw.length < 40) return null;
    const k = this.derive('aead', domain), nonce = raw.slice(0, 24), ct = raw.slice(24, raw.length - 16), tag = raw.slice(raw.length - 16);
    if (hex(hmac(k, concat(nonce, ct, enc(domain))).slice(0, 16)) !== hex(tag)) return null;
    const ks = keystream(k, nonce, ct.length); return dec(ct.map((x, i) => x ^ ks[i]));
  }
}

/* ------------------------------------------------------------------ tokenizer */

function tokenize(src, base) {
  base = base || 0;
  const toks = []; let i = 0; const n = src.length;
  const push = (t, v, s, e, extra) => toks.push(Object.assign({ t, v, u: typeof v === 'string' ? v.toUpperCase() : v, s: s + base, e: e + base }, extra || {}));
  while (i < n) {
    const c = src[i];
    if (/\s/.test(c)) { i++; continue; }
    if (c === '-' && src[i + 1] === '-') { while (i < n && src[i] !== '\n') i++; continue; }
    if (c === '/' && src[i + 1] === '*') { const j = src.indexOf('*/', i + 2); i = j < 0 ? n : j + 2; continue; }
    const s = i;
    if (c === "'") {
      let v = ''; i++;
      for (;;) {
        if (i >= n) throw sqlerr('42601', 'unterminated quoted string');
        if (src[i] === "'") { if (src[i + 1] === "'") { v += "'"; i += 2; continue; } i++; break; }
        v += src[i++];
      }
      push('str', v, s, i); continue;
    }
    if (c === '"') {
      const j = src.indexOf('"', i + 1); if (j < 0) throw sqlerr('42601', 'unterminated quoted identifier');
      push('id', src.slice(i + 1, j), s, j + 1, { quoted: true }); i = j + 1; continue;
    }
    if (c === '$') {
      const m = /^\$(\d+)/.exec(src.slice(i));
      if (m) { push('param', Number(m[1]), s, i + m[0].length); i += m[0].length; continue; }
      const t = /^\$([A-Za-z_]\w*)?\$/.exec(src.slice(i));
      if (t) {
        const tag = t[0]; const j = src.indexOf(tag, i + tag.length);
        if (j < 0) throw sqlerr('42601', 'unterminated dollar-quoted string');
        push('dollar', src.slice(i + tag.length, j), s, j + tag.length, { bodyStart: i + tag.length + base });
        i = j + tag.length; continue;
      }
    }
    if (/[A-Za-z_]/.test(c)) { let j = i + 1; while (j < n && /[\w$]/.test(src[j])) j++; push('id', src.slice(i, j), s, j); i = j; continue; }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1] || ''))) {
      let j = i; while (j < n && /[0-9.]/.test(src[j])) j++;
      if (/[eE]/.test(src[j] || '') && /[-+0-9]/.test(src[j + 1] || '')) { j += 2; while (j < n && /[0-9]/.test(src[j])) j++; }
      push('num', src.slice(i, j), s, j); i = j; continue;
    }
    if (c === '\\') { let j = i; while (j < n && src[j] !== '\n') j++; push('meta', src.slice(i, j).trim(), s, j); i = j; continue; }
    const two = src.slice(i, i + 2);
    if (['<>', '!=', '<=', '>=', '::', '||'].includes(two)) { push('op', two, s, i + 2); i += 2; continue; }
    if ('=<>+-*/%(),;.[]'.includes(c)) { push('op', c, s, i + 1); i++; continue; }
    throw sqlerr('42601', `syntax error at or near "${c}"`);
  }
  return toks;
}

/* ------------------------------------------------------------------ parser */

const RESERVED = new Set(['SELECT','FROM','WHERE','JOIN','INNER','LEFT','RIGHT','FULL','CROSS','OUTER','NATURAL','ON','USING','GROUP','ORDER','HAVING','LIMIT','OFFSET','UNION','EXCEPT','INTERSECT','AND','OR','NOT','AS','SET','VALUES','RETURNING','INTO','WITH','WHEN','THEN','ELSE','END','IS','IN','LIKE','ILIKE','BETWEEN','BY','ASC','DESC','CASE','DISTINCT','ALL','EXISTS','NULL','TRUE','FALSE','OVER','WINDOW','FETCH']);
const AGGREGATES = new Set(['count', 'sum', 'avg', 'min', 'max', 'string_agg', 'bool_and', 'bool_or', 'array_agg']);
const WINDOW_ONLY = new Set(['row_number', 'rank', 'dense_rank', 'percent_rank', 'cume_dist', 'ntile', 'lag', 'lead', 'first_value', 'last_value', 'nth_value']);

class Parser {
  constructor(toks) { this.t = toks; this.i = 0; }
  peek(k) { return this.t[this.i + (k || 0)]; }
  eof() { return this.i >= this.t.length; }
  next() { const x = this.t[this.i++]; if (!x) throw sqlerr('42601', 'syntax error at end of input'); return x; }
  isKw(w, k) { const x = this.peek(k); return !!x && x.t === 'id' && !x.quoted && x.u === w; }
  isOp(o, k) { const x = this.peek(k); return !!x && x.t === 'op' && x.v === o; }
  acceptKw(w) { if (this.isKw(w)) { this.i++; return true; } return false; }
  acceptOp(o) { if (this.isOp(o)) { this.i++; return true; } return false; }
  expectKw(w) { if (!this.acceptKw(w)) this.fail(); }
  expectOp(o) { if (!this.acceptOp(o)) this.fail(); }
  fail() {
    const x = this.peek();
    throw sqlerr('42601', x ? `syntax error at or near "${x.t === 'str' ? "'" + x.v + "'" : x.t === 'param' ? '$' + x.v : x.v}"` : 'syntax error at end of input');
  }
  ident() { const x = this.next(); if (x.t !== 'id') { this.i--; this.fail(); } return x.quoted ? x.v : x.v.toLowerCase(); }
  identNotReserved() { const x = this.peek(); if (!x || x.t !== 'id' || (!x.quoted && RESERVED.has(x.u))) return null; this.i++; return x.quoted ? x.v : x.v.toLowerCase(); }
  qualifiedName() { let n = this.ident(); while (this.isOp('.') && this.peek(1) && this.peek(1).t === 'id') { this.i++; n = this.ident(); } return n; }
  identList() { const v = []; do v.push(this.ident()); while (this.acceptOp(',')); return v; }
  parenIdents() { this.expectOp('('); const v = this.identList(); this.expectOp(')'); return v; }
  skipParens() { let depth = 0; do { const x = this.next(); if (x.t === 'op' && x.v === '(') depth++; else if (x.t === 'op' && x.v === ')') depth--; } while (depth > 0); }
  rest() { const out = []; while (!this.eof()) out.push(this.next()); return out; }
  typeName() {
    let ty = this.ident().toUpperCase();
    if (ty === 'DOUBLE' && this.acceptKw('PRECISION')) ty = 'DOUBLE PRECISION';
    else if (ty === 'CHARACTER' && this.acceptKw('VARYING')) ty = 'CHARACTER VARYING';
    else if ((ty === 'TIMESTAMP' || ty === 'TIME') && (this.isKw('WITH') || this.isKw('WITHOUT')) && this.isKw('TIME', 1)) {
      const w = this.next().u; this.expectKw('TIME'); this.expectKw('ZONE'); ty += ` ${w} TIME ZONE`;
    }
    if (this.acceptOp('(')) { const a = []; while (!this.isOp(')')) { const x = this.next(); if (!(x.t === 'op' && x.v === ',')) a.push(x.v); } this.expectOp(')'); ty += '(' + a.join(', ') + ')'; }
    return ty;
  }

  /* --- statements */
  statement() {
    const x = this.peek();
    if (!x) return null;
    if (x.t === 'meta') { this.i++; return { type: 'meta', text: x.v }; }
    if (this.isKw('SELECT') || this.isKw('WITH') || this.isOp('(') || this.isKw('VALUES')) return this.query();
    const w = x.t === 'id' && !x.quoted ? x.u : null;
    switch (w) {
      case 'INSERT': this.i++; return this.insert();
      case 'UPDATE': this.i++; return this.update();
      case 'DELETE': this.i++; return this.del();
      case 'CREATE': this.i++; return this.create();
      case 'ALTER': this.i++; return this.alter();
      case 'DROP': this.i++; return this.drop();
      case 'GRANT': case 'REVOKE': this.i++; return this.grant(w === 'GRANT');
      case 'COMMENT': this.i++; return this.comment();
      case 'SET': this.i++; return this.set();
      case 'RESET': this.i++; return this.reset();
      case 'SHOW': this.i++; return { type: 'show', words: this.rest().map((t) => (t.t === 'id' ? t.v : String(t.v))) };
      case 'EXPLAIN': { this.i++; while (this.acceptKw('ANALYZE') || this.acceptKw('VERBOSE')); return { type: 'explain', inner: this.statement() }; }
      case 'BEGIN': this.i++; this.rest(); return { type: 'begin' };
      case 'START': this.i++; this.expectKw('TRANSACTION'); this.rest(); return { type: 'begin' };
      case 'COMMIT': case 'END': this.i++; this.acceptKw('TRANSACTION') || this.acceptKw('WORK'); return { type: 'commit' };
      case 'ROLLBACK': case 'ABORT': {
        this.i++; this.acceptKw('TRANSACTION') || this.acceptKw('WORK');
        if (w === 'ROLLBACK' && this.acceptKw('TO')) { this.acceptKw('SAVEPOINT'); if (this.eof()) throw sqlerr('42601', 'a savepoint name is needed'); return { type: 'rollbackTo', name: this.ident() }; }
        return { type: 'rollback' };
      }
      case 'SAVEPOINT': this.i++; if (this.eof()) throw sqlerr('42601', 'a savepoint name is needed'); return { type: 'savepoint', name: this.ident() };
      case 'RELEASE': this.i++; this.acceptKw('SAVEPOINT'); if (this.eof()) throw sqlerr('42601', 'a savepoint name is needed'); return { type: 'release', name: this.ident() };
      case 'REINDEX': { this.i++; const k = this.peek(); const n = this.peek(1); if (!k || !n || this.peek(2) || !['TABLE', 'INDEX'].includes(k.u)) throw sqlerr('42601', 'REINDEX TABLE name, or REINDEX INDEX name'); this.i += 2; return { type: 'reindex', kind: k.u, name: n.quoted ? n.v : String(n.v).toLowerCase() }; }
      case 'COPY': this.rest(); return { type: 'copy' };
      case 'TRUNCATE': case 'MERGE': this.rest(); return { type: 'otherStmt', word: w };
    }
    this.fail();
  }

  query() {
    const withs = [];
    if (this.acceptKw('WITH')) {
      if (this.isKw('RECURSIVE')) throw unsupported('WITH RECURSIVE is not supported.');
      do {
        const name = this.ident(); let cols = null;
        if (this.isOp('(')) cols = this.parenIdents();
        this.expectKw('AS'); this.expectOp('('); const q = this.query(); this.expectOp(')');
        withs.push({ name, q, cols });
      } while (this.acceptOp(','));
    }
    let q = this.setTerm();
    while (this.isKw('UNION') || this.isKw('EXCEPT') || this.isKw('INTERSECT')) {
      const op = this.next().u; let all = false;
      if (this.acceptKw('ALL')) all = true; else this.acceptKw('DISTINCT');
      q = { type: 'compound', op, all, l: q, r: this.setTerm(), orderBy: [], limit: null, offset: null, with: [] };
    }
    q = this.tail(q);
    if (withs.length) q.with = withs.concat(q.with || []);
    return q;
  }
  setTerm() {
    if (this.acceptOp('(')) { const q = this.query(); this.expectOp(')'); return q; }
    if (this.acceptKw('VALUES')) {
      const rows = [];
      do { this.expectOp('('); const r = []; do r.push(this.expr()); while (this.acceptOp(',')); this.expectOp(')'); rows.push(r); } while (this.acceptOp(','));
      return { type: 'values', rows, orderBy: [], limit: null, offset: null, with: [] };
    }
    return this.select();
  }
  tail(q) {
    if (!(this.isKw('ORDER') || this.isKw('LIMIT') || this.isKw('OFFSET') || this.isKw('FETCH'))) return q;
    if ((q.orderBy && q.orderBy.length) || q.limit || q.offset) q = { type: 'select', distinct: false, cols: [{ e: { type: 'star', table: null } }], from: [{ kind: 'sub', q, alias: '__dep_q', join: null }], where: null, groupBy: [], having: null, orderBy: [], limit: null, offset: null, with: [] };
    if (this.acceptKw('ORDER')) {
      this.expectKw('BY');
      do { const e = this.expr(); let dir = 'ASC'; if (this.acceptKw('DESC')) dir = 'DESC'; else this.acceptKw('ASC'); let nulls = null; if (this.acceptKw('NULLS')) nulls = this.next().u; q.orderBy.push({ e, dir, nulls }); } while (this.acceptOp(','));
    }
    for (;;) {
      if (this.acceptKw('LIMIT')) { if (!this.acceptKw('ALL')) q.limit = this.expr(); continue; }
      if (this.acceptKw('OFFSET')) { q.offset = this.expr(); this.acceptKw('ROWS') || this.acceptKw('ROW'); continue; }
      if (this.acceptKw('FETCH')) { this.acceptKw('FIRST') || this.acceptKw('NEXT'); q.limit = this.isKw('ROW') || this.isKw('ROWS') ? { type: 'lit', ltype: 'num', v: 1 } : this.expr(); this.acceptKw('ROWS') || this.acceptKw('ROW'); this.expectKw('ONLY'); continue; }
      break;
    }
    return q;
  }

  select() {
    this.expectKw('SELECT');
    const q = { type: 'select', distinct: false, cols: [], from: [], where: null, groupBy: [], having: null, orderBy: [], limit: null, offset: null, with: [] };
    if (this.acceptKw('DISTINCT')) { if (this.isKw('ON')) throw unsupported('DISTINCT ON is not supported.'); q.distinct = true; } else this.acceptKw('ALL');
    do {
      if (this.isOp('*')) { this.i++; q.cols.push({ e: { type: 'star', table: null } }); continue; }
      if (this.peek() && this.peek().t === 'id' && this.isOp('.', 1) && this.isOp('*', 2)) { const tb = this.ident(); this.i += 2; q.cols.push({ e: { type: 'star', table: tb } }); continue; }
      const e = this.expr(); let alias = null;
      if (this.acceptKw('AS')) alias = this.ident(); else alias = this.identNotReserved();
      q.cols.push({ e, alias });
    } while (this.acceptOp(','));
    if (this.acceptKw('FROM')) q.from = this.fromList();
    if (this.acceptKw('WHERE')) q.where = this.expr();
    if (this.acceptKw('GROUP')) { this.expectKw('BY'); do q.groupBy.push(this.expr()); while (this.acceptOp(',')); }
    if (this.acceptKw('HAVING')) q.having = this.expr();
    if (this.isKw('WINDOW')) throw unsupported('Named windows are not supported; write the window in OVER (...).');
    return q;
  }

  fromList() {
    const items = [this.fromItem(null)];
    for (;;) {
      if (this.acceptOp(',')) { items.push(this.fromItem('CROSS')); continue; }
      let jt = null;
      if (this.isKw('NATURAL')) throw unsupported('NATURAL JOIN is not supported; name the join columns with USING or ON.');
      if (this.acceptKw('JOIN')) jt = 'INNER';
      else if (this.isKw('INNER') && this.isKw('JOIN', 1)) { this.i += 2; jt = 'INNER'; }
      else if (this.isKw('LEFT') || this.isKw('RIGHT') || this.isKw('FULL')) { jt = this.next().u; this.acceptKw('OUTER'); this.expectKw('JOIN'); }
      else if (this.isKw('CROSS')) { this.i++; this.expectKw('JOIN'); jt = 'CROSS'; }
      if (!jt) break;
      const it = this.fromItem(jt);
      if (jt !== 'CROSS') {
        if (this.acceptKw('ON')) it.on = this.expr();
        else if (this.acceptKw('USING')) it.using = this.parenIdents();
        else this.fail();
      }
      items.push(it);
    }
    return items;
  }

  fromItem(join) {
    const it = { join, on: null, using: null, alias: null, colAliases: null };
    if (this.isKw('LATERAL')) throw unsupported('LATERAL is not supported.');
    if (this.acceptOp('(')) {
      if (this.isKw('SELECT') || this.isKw('WITH') || this.isKw('VALUES') || this.isOp('(')) { it.kind = 'sub'; it.q = this.query(); this.expectOp(')'); }
      else { const inner = this.fromList(); this.expectOp(')'); if (inner.length !== 1) throw unsupported('Parenthesized joins are not supported.'); return Object.assign(inner[0], { join }); }
    } else {
      const name = this.qualifiedName();
      if (this.acceptOp('(')) {
        it.kind = 'func'; it.name = name; it.args = [];
        if (!this.isOp(')')) do it.args.push(this.expr()); while (this.acceptOp(','));
        this.expectOp(')');
      } else { it.kind = 'table'; it.name = name; }
    }
    if (this.acceptKw('AS')) it.alias = this.ident(); else { const a = this.identNotReserved(); if (a) it.alias = a; }
    if (it.alias && this.isOp('(')) it.colAliases = this.parenIdents();
    if (it.kind === 'sub' && !it.alias) throw sqlerr('42601', 'subquery in FROM must have an alias');
    return it;
  }

  /* --- expressions */
  expr() { return this.orExpr(); }
  orExpr() { let l = this.andExpr(); while (this.acceptKw('OR')) l = { type: 'bin', op: 'OR', l, r: this.andExpr() }; return l; }
  andExpr() { let l = this.notExpr(); while (this.acceptKw('AND')) l = { type: 'bin', op: 'AND', l, r: this.notExpr() }; return l; }
  notExpr() { if (this.acceptKw('NOT')) return { type: 'not', e: this.notExpr() }; return this.cmpExpr(); }
  cmpExpr() {
    let l = this.addExpr();
    for (;;) {
      const x = this.peek();
      if (x && x.t === 'op' && ['=', '<>', '!=', '<', '>', '<=', '>='].includes(x.v)) { this.i++; l = { type: 'bin', op: x.v === '!=' ? '<>' : x.v, l, r: this.addExpr() }; continue; }
      if (this.isKw('IS')) {
        this.i++; const neg = this.acceptKw('NOT');
        if (this.acceptKw('NULL')) { l = { type: 'isnull', e: l, neg }; continue; }
        if (this.acceptKw('TRUE') || this.acceptKw('FALSE')) { l = { type: 'istrue', e: l, neg, v: this.t[this.i - 1].u === 'TRUE' }; continue; }
        if (this.acceptKw('DISTINCT')) { this.expectKw('FROM'); l = { type: 'distinctfrom', l, r: this.addExpr(), neg }; continue; }
        this.fail();
      }
      let neg = false;
      if (this.isKw('NOT') && (this.isKw('IN', 1) || this.isKw('LIKE', 1) || this.isKw('BETWEEN', 1) || this.isKw('ILIKE', 1))) { this.i++; neg = true; }
      if (this.acceptKw('IN')) {
        this.expectOp('(');
        if (this.isKw('SELECT') || this.isKw('WITH')) { const q = this.query(); this.expectOp(')'); l = { type: 'insub', e: l, q, neg }; }
        else { const list = []; do list.push(this.expr()); while (this.acceptOp(',')); this.expectOp(')'); l = { type: 'inlist', e: l, list, neg }; }
        continue;
      }
      if (this.isKw('LIKE') || this.isKw('ILIKE')) { const ci = this.next().u === 'ILIKE'; l = { type: 'like', e: l, pat: this.addExpr(), neg, ci }; continue; }
      if (this.acceptKw('BETWEEN')) { const lo = this.addExpr(); this.expectKw('AND'); const hi = this.addExpr(); l = { type: 'between', e: l, lo, hi, neg }; continue; }
      break;
    }
    return l;
  }
  addExpr() {
    let l = this.mulExpr();
    for (;;) { const x = this.peek(); if (x && x.t === 'op' && ['+', '-', '||'].includes(x.v)) { this.i++; l = { type: 'arith', op: x.v, l, r: this.mulExpr() }; } else break; }
    return l;
  }
  mulExpr() {
    let l = this.unary();
    for (;;) { const x = this.peek(); if (x && x.t === 'op' && ['*', '/', '%'].includes(x.v)) { this.i++; l = { type: 'arith', op: x.v, l, r: this.unary() }; } else break; }
    return l;
  }
  unary() { if (this.acceptOp('-')) return { type: 'neg', e: this.unary() }; if (this.acceptOp('+')) return this.unary(); return this.postfix(); }
  postfix() { let e = this.primary(); while (this.acceptOp('::')) e = { type: 'cast', e, to: this.typeName() }; return e; }
  primary() {
    const x = this.next();
    if (x.t === 'str') return { type: 'lit', ltype: 'str', v: x.v, tok: x };
    if (x.t === 'num') { const isF = /[.eE]/.test(x.v); return { type: 'lit', ltype: isF ? 'float' : 'num', v: Number(x.v), raw: x.v, tok: x }; }
    if (x.t === 'param') return { type: 'param', n: x.v, tok: x };
    if (x.t === 'op' && x.v === '(') {
      if (this.isKw('SELECT') || this.isKw('WITH')) { const q = this.query(); this.expectOp(')'); return { type: 'scalarsub', q }; }
      const e = this.expr(); this.expectOp(')'); return e;
    }
    if (x.t !== 'id') { this.i--; this.fail(); }
    if (!x.quoted) {
      const nxt = this.peek();
      if ((x.u === 'DATE' || x.u === 'TIMESTAMP' || x.u === 'TIMESTAMPTZ') && nxt && nxt.t === 'str') { this.i++; return { type: 'cast', e: { type: 'lit', ltype: 'str', v: nxt.v, tok: nxt }, to: x.u === 'DATE' ? 'DATE' : 'TIMESTAMP' }; }
      if (x.u === 'INTERVAL' && nxt && nxt.t === 'str') { this.i++; let text = nxt.v; const unit = this.peek(); if (unit && unit.t === 'id' && /^(YEAR|MONTH|WEEK|DAY|HOUR|MINUTE|SECOND)S?$/.test(unit.u)) { this.i++; text += ' ' + unit.v; } return { type: 'interval', text }; }
      switch (x.u) {
        case 'NULL': return { type: 'lit', ltype: 'null', v: null, tok: x };
        case 'TRUE': return { type: 'lit', ltype: 'bool', v: true, tok: x };
        case 'FALSE': return { type: 'lit', ltype: 'bool', v: false, tok: x };
        case 'CURRENT_DATE': return { type: 'curdate' };
        case 'CURRENT_TIMESTAMP': case 'LOCALTIMESTAMP': return { type: 'curts' };
        case 'CURRENT_USER': case 'SESSION_USER': case 'CURRENT_ROLE': case 'USER': if (!this.isOp('(')) return { type: 'curuser', which: x.u }; break;
        case 'EXISTS': { this.expectOp('('); const q = this.query(); this.expectOp(')'); return { type: 'exists', q }; }
        case 'CAST': { this.expectOp('('); const e = this.expr(); this.expectKw('AS'); const to = this.typeName(); this.expectOp(')'); return { type: 'cast', e, to }; }
        case 'EXTRACT': { this.expectOp('('); const field = this.ident().toLowerCase(); this.expectKw('FROM'); const e = this.expr(); this.expectOp(')'); return { type: 'func', name: 'date_part', args: [{ type: 'lit', ltype: 'str', v: field }, e], star: false, distinct: false }; }
        case 'CASE': {
          const whens = []; let base = null; if (!this.isKw('WHEN')) base = this.expr();
          while (this.acceptKw('WHEN')) { const c = this.expr(); this.expectKw('THEN'); whens.push({ c, v: this.expr() }); }
          let els = null; if (this.acceptKw('ELSE')) els = this.expr(); this.expectKw('END');
          return { type: 'case', base, whens, els };
        }
      }
      if (RESERVED.has(x.u) && !((x.u === 'LEFT' || x.u === 'RIGHT') && this.isOp('('))) { this.i--; this.fail(); }
    }
    const name = x.quoted ? x.v : x.v.toLowerCase();
    if (this.isOp('(')) {
      this.i++; const f = { type: 'func', name, args: [], star: false, distinct: false, tok: x, over: null };
      if (this.acceptOp('*')) f.star = true;
      else if (!this.isOp(')')) { if (this.acceptKw('DISTINCT')) f.distinct = true; else this.acceptKw('ALL'); do f.args.push(this.expr()); while (this.acceptOp(',')); if (this.isKw('ORDER')) throw unsupported('ORDER BY inside an aggregate is not supported.'); }
      this.expectOp(')');
      if (this.isKw('FILTER')) throw unsupported('FILTER (WHERE ...) on aggregates is not supported.');
      if (this.acceptKw('OVER')) {
        this.expectOp('('); const over = { partition: [], order: [] };
        if (this.acceptKw('PARTITION')) { this.expectKw('BY'); do over.partition.push(this.expr()); while (this.acceptOp(',')); }
        if (this.acceptKw('ORDER')) { this.expectKw('BY'); do { const e = this.expr(); let dir = 'ASC'; if (this.acceptKw('DESC')) dir = 'DESC'; else this.acceptKw('ASC'); over.order.push({ e, dir }); } while (this.acceptOp(',')); }
        if (this.isKw('ROWS') || this.isKw('RANGE') || this.isKw('GROUPS')) { const s = this.i; while (!this.isOp(')')) this.next(); over.frame = this.t.slice(s, this.i).map((t) => t.v).join(' '); }
        this.expectOp(')'); f.over = over;
      }
      return f;
    }
    if (this.acceptOp('.')) {
      const col = this.ident();
      if (this.acceptOp('.')) { const c2 = this.ident(); return { type: 'col', table: col, name: c2 }; }
      return { type: 'col', table: name, name: col };
    }
    return { type: 'col', table: null, name };
  }

  /* --- DML */
  insert() {
    this.expectKw('INTO'); const table = this.qualifiedName(); let cols = null;
    if (this.acceptKw('AS')) this.ident();
    if (this.isOp('(') && !(this.isKw('SELECT', 1) || this.isKw('WITH', 1) || this.isKw('VALUES', 1))) cols = this.parenIdents();
    if (this.acceptKw('DEFAULT')) { this.expectKw('VALUES'); throw unsupported('DEFAULT values are not supported.'); }
    const q = this.query();
    if (this.isKw('ON') || this.isKw('RETURNING')) { this.rest(); return { type: 'insert', table, cols, q, bad: 'INSERT ... ON CONFLICT and RETURNING are not supported.' }; }
    return { type: 'insert', table, cols, q };
  }
  update() {
    const table = this.qualifiedName(); let alias = null; if (this.acceptKw('AS')) alias = this.ident(); else alias = this.identNotReserved();
    this.expectKw('SET'); const sets = [];
    do {
      if (this.isOp('(')) throw unsupported('Tuple assignments are not supported.');
      const col = this.qualifiedName(); this.expectOp('='); sets.push({ col, e: this.expr() });
    } while (this.acceptOp(','));
    let bad = null;
    if (this.isKw('FROM')) { bad = 'UPDATE ... FROM and RETURNING are not supported.'; this.rest(); }
    let where = null; if (!bad && this.acceptKw('WHERE')) where = this.expr();
    if (!bad && this.isKw('RETURNING')) { bad = 'UPDATE ... FROM and RETURNING are not supported.'; this.rest(); }
    return { type: 'update', table, alias, sets, where, bad };
  }
  del() {
    this.acceptKw('FROM'); const table = this.qualifiedName(); let alias = null; if (this.acceptKw('AS')) alias = this.ident(); else alias = this.identNotReserved();
    let bad = null;
    if (this.isOp(',') || this.isKw('USING')) { bad = 'DELETE from one table only.'; this.rest(); }
    let where = null; if (!bad && this.acceptKw('WHERE')) where = this.expr();
    if (!bad && this.isKw('RETURNING')) { bad = 'DELETE from one table only.'; this.rest(); }
    return { type: 'delete', table, alias, where, bad };
  }

  /* --- DDL */
  create() {
    let orReplace = false;
    if (this.acceptKw('OR')) { this.expectKw('REPLACE'); orReplace = true; }
    if (this.acceptKw('FUNCTION')) return this.createFunction(orReplace);
    let temporary = false;
    if (this.acceptKw('TEMPORARY') || this.acceptKw('TEMP')) temporary = true;
    if (this.acceptKw('TABLE')) return this.createTable(temporary);
    if (this.isKw('UNIQUE') || this.isKw('INDEX')) return this.createIndex();
    if (this.acceptKw('ROLE') || this.acceptKw('USER')) return this.roleOptions('CREATE', this.t[this.i - 1].u);
    if (this.acceptKw('ENDPOINT')) { const name = this.ident(); this.expectKw('MODE'); const mode = this.ident().toUpperCase(); if (!this.eof()) this.fail(); return { type: 'createEndpoint', name, mode }; }
    this.rest(); return { type: 'createOther' };
  }
  createTable(temporary) {
    const t = { type: 'createTable', temporary, ifNotExists: false, name: null, cols: [], tcons: [], asQuery: false };
    if (this.acceptKw('IF')) { this.expectKw('NOT'); this.expectKw('EXISTS'); t.ifNotExists = true; }
    t.name = this.qualifiedName();
    if (this.acceptKw('AS')) { this.rest(); t.asQuery = true; return t; }
    this.expectOp('(');
    do {
      if (this.acceptKw('CONSTRAINT')) this.ident();
      if (this.isKw('PRIMARY')) { this.i++; this.expectKw('KEY'); t.tcons.push({ k: 'pk', cols: this.parenIdents() }); continue; }
      if (this.isKw('UNIQUE')) { this.i++; t.tcons.push({ k: 'unique', cols: this.parenIdents() }); continue; }
      if (this.isKw('FOREIGN')) { this.i++; this.expectKw('KEY'); this.skipParens(); this.expectKw('REFERENCES'); this.qualifiedName(); if (this.isOp('(')) this.skipParens(); this.fkActions(); t.tcons.push({ k: 'fk' }); continue; }
      if (this.isKw('CHECK')) { this.i++; const s = this.i; this.skipParens(); t.tcons.push({ k: 'check', text: 'CHECK ' + this.t.slice(s, this.i).map((x) => x.v).join(' ') }); continue; }
      t.cols.push(this.columnDef());
    } while (this.acceptOp(','));
    this.expectOp(')');
    if (this.acceptKw('AS')) { this.rest(); t.asQuery = true; }
    if (!this.eof()) this.fail();
    return t;
  }
  fkActions() {
    const acts = [];
    while (this.isKw('ON')) {
      this.i++; const which = this.next().u; let a = this.next().u;
      if (a === 'SET' || a === 'NO') a += ' ' + this.next().u;
      acts.push({ which, a });
    }
    return acts;
  }
  columnDef() {
    const c = { name: this.ident(), typeText: this.typeName(), protected: false, domain: null, opts: [] };
    for (;;) {
      if (this.acceptKw('PROTECTED')) { c.protected = true; if (this.acceptKw('MATCH')) { this.expectKw('DOMAIN'); c.domain = this.ident(); } continue; }
      if (this.acceptKw('CONSTRAINT')) { this.ident(); continue; }
      if (this.acceptKw('PRIMARY')) { this.expectKw('KEY'); c.opts.push({ k: 'pk' }); continue; }
      if (this.acceptKw('UNIQUE')) { c.opts.push({ k: 'unique' }); continue; }
      if (this.isKw('NOT') && this.isKw('NULL', 1)) { this.i += 2; c.opts.push({ k: 'notnull' }); continue; }
      if (this.acceptKw('NULL')) continue;
      if (this.acceptKw('DEFAULT')) { this.unary(); c.opts.push({ k: 'default' }); continue; }
      if (this.acceptKw('CHECK')) { this.skipParens(); c.opts.push({ k: 'check' }); continue; }
      if (this.acceptKw('REFERENCES')) {
        const table = this.qualifiedName(); let col = null;
        if (this.acceptOp('(')) { col = this.ident(); this.expectOp(')'); }
        c.opts.push({ k: 'ref', table, col, actions: this.fkActions() }); continue;
      }
      break;
    }
    return c;
  }
  createIndex() {
    const unique = this.acceptKw('UNIQUE'); this.expectKw('INDEX'); this.acceptKw('CONCURRENTLY');
    let ifNotExists = false;
    if (this.acceptKw('IF')) { this.expectKw('NOT'); this.expectKw('EXISTS'); ifNotExists = true; }
    let name = null; if (!this.isKw('ON')) name = this.ident();
    this.expectKw('ON'); this.acceptKw('ONLY'); const table = this.qualifiedName();
    if (this.acceptKw('USING')) this.ident();
    this.expectOp('('); const cols = []; let bad = null;
    do {
      const x = this.peek();
      if (x && x.t === 'id' && (this.isOp(',', 1) || this.isOp(')', 1) || this.isKw('ASC', 1) || this.isKw('DESC', 1) || this.isKw('NULLS', 1))) cols.push(this.ident());
      else { const s = this.i; this.expr(); bad = bad || tokText(this.t.slice(s, this.i)); }
      this.acceptKw('ASC') || this.acceptKw('DESC'); if (this.acceptKw('NULLS')) this.next();
    } while (this.acceptOp(','));
    this.expectOp(')');
    let partial = false;
    if (this.acceptKw('INCLUDE')) { this.skipParens(); partial = true; }
    if (this.acceptKw('WHERE')) { this.expr(); partial = true; }
    if (!this.eof()) this.fail();
    return { type: 'createIndex', unique, ifNotExists, name, table, cols, exprCol: bad, partial };
  }
  createFunction(orReplace) {
    const name = this.qualifiedName(); this.expectOp('('); const params = [];
    if (!this.isOp(')')) do { const pn = this.ident(); const pt = this.typeName(); params.push({ name: pn, typeText: pt }); } while (this.acceptOp(','));
    this.expectOp(')'); this.expectKw('RETURNS');
    if (!this.acceptKw('TABLE')) throw unsupported('Declared functions must use RETURNS TABLE (...).');
    this.expectOp('('); const returns = [];
    do returns.push({ name: this.ident(), typeText: this.typeName() }); while (this.acceptOp(','));
    this.expectOp(')');
    const f = { type: 'createFunction', orReplace, name, params, returns, language: null, discloses: [], links: [], body: '' };
    while (!this.eof()) {
      if (this.acceptKw('LANGUAGE')) { f.language = this.ident(); continue; }
      if (this.acceptKw('DISCLOSES')) { f.discloses.push(...this.identList()); continue; }
      if (this.acceptKw('LINKS')) { f.links.push(...this.identList()); continue; }
      if (this.acceptKw('AS')) { const b = this.next(); if (b.t !== 'dollar' && b.t !== 'str') { this.i--; this.fail(); } f.body = b.v; continue; }
      if (this.acceptKw('STABLE') || this.acceptKw('IMMUTABLE') || this.acceptKw('VOLATILE') || this.acceptKw('STRICT')) continue;
      this.fail();
    }
    if (!f.body.trim()) throw sqlerr('42P13', 'function body is missing (AS $$ ... $$)');
    return f;
  }
  roleOptions(verb, kind) {
    const name = this.ident(); const o = {};
    if (kind === 'USER' && verb === 'CREATE') o.login = true;
    this.acceptKw('WITH');
    while (!this.eof()) {
      if (this.acceptKw('LOGIN')) { o.login = true; continue; }
      if (this.acceptKw('NOLOGIN')) { o.login = false; continue; }
      if (this.acceptKw('SUPERUSER')) { o.superuser = true; continue; }
      if (this.acceptKw('NOSUPERUSER')) { o.superuser = false; continue; }
      if (this.acceptKw('MATCH')) {
        if (!this.acceptKw('LIMIT')) this.fail();
        if (this.acceptKw('NONE')) { o.matchLimit = { kind: 'none' }; continue; }
        if (this.acceptKw('DEFAULT')) { o.matchLimit = { kind: 'default' }; continue; }
        const n = this.next(); if (n.t !== 'num') { this.i--; this.fail(); }
        const count = Number(n.v); if (!Number.isInteger(count) || count < 0) throw sqlerr('22023', `invalid match limit "${n.v}"`);
        if (!this.acceptKw('PER')) this.fail();
        const unit = this.ident(); const per = unitSecs(unit);
        if (!per) throw sqlerr('22023', `unknown period "${unit}"`, 'Use SECOND, MINUTE, HOUR or DAY.');
        o.matchLimit = { kind: 'limit', count, per }; continue;
      }
      if (this.acceptKw('PASSWORD')) { const p = this.next(); if (p.t === 'str') o.password = p.v; else if (p.t === 'id' && p.u === 'NULL') o.password = null; else { this.i--; this.fail(); } continue; }
      this.fail();
    }
    return { type: 'role', verb, name, o };
  }
  alter() {
    if (this.acceptKw('ROLE') || this.acceptKw('USER')) return this.roleOptions('ALTER', this.t[this.i - 1].u);
    if (this.acceptKw('SYSTEM')) { if (this.acceptKw('ROTATE') && this.acceptKw('MASTER') && this.acceptKw('KEY') && this.eof()) return { type: 'rotateKey' }; this.rest(); return { type: 'alterOther' }; }
    this.rest(); return { type: 'alterOther' };
  }
  grant(isGrant) {
    const g = { type: 'grant', grant: isGrant, privs: [], object: null, roles: [] };
    do {
      const w = this.ident().toUpperCase(); let cols = null;
      if ((w === 'SELECT' || w === 'UPDATE') && this.isOp('(')) cols = this.parenIdents();
      if (w === 'ALL') this.acceptKw('PRIVILEGES');
      if (!['SELECT', 'UPDATE', 'INSERT', 'DELETE', 'ALL', 'EXECUTE', 'DISCLOSE', 'LINK', 'CONNECT'].includes(w)) throw sqlerr('42601', `unrecognized privilege type "${w.toLowerCase()}"`);
      g.privs.push({ p: w, cols });
    } while (this.acceptOp(','));
    this.expectKw('ON');
    if (this.acceptKw('FUNCTION')) { const n = this.qualifiedName(); if (this.isOp('(')) this.skipParens(); g.object = { kind: 'function', name: n }; }
    else if (this.acceptKw('MATCH')) { this.expectKw('DOMAIN'); g.object = { kind: 'domain', name: this.ident() }; }
    else if (this.acceptKw('ENDPOINT')) g.object = { kind: 'endpoint', name: this.ident() };
    else { this.acceptKw('TABLE'); g.object = { kind: 'table', names: this.identList() }; }
    if (isGrant) this.expectKw('TO'); else this.expectKw('FROM');
    g.roles = this.identList();
    if (!this.eof()) this.fail();
    return g;
  }
  comment() {
    this.expectKw('ON'); this.expectKw('FUNCTION'); const fn = this.qualifiedName(); if (this.isOp('(')) this.skipParens();
    this.expectKw('IS'); const v = this.next(); if (v.t !== 'str' && !(v.t === 'id' && v.u === 'NULL')) { this.i--; this.fail(); }
    return { type: 'comment', fn, text: v.t === 'str' ? v.v : null };
  }
  set() {
    if (this.acceptKw('ROLE')) { const r = this.next(); return { type: 'setRole', role: r.t === 'str' ? r.v.toLowerCase() : r.quoted ? r.v : String(r.v).toLowerCase() }; }
    if (this.acceptKw('SESSION')) {
      if (this.acceptKw('AUTHORIZATION')) { const r = this.next(); return { type: 'setRole', role: r.t === 'str' ? r.v.toLowerCase() : String(r.v).toLowerCase() }; }
    } else this.acceptKw('LOCAL');
    const toks = this.rest();
    const name = toks.length >= 3 && toks[1].t === 'op' && toks[1].v === '.' ? (String(toks[0].v) + '.' + String(toks[2].v)).toLowerCase() : toks.length ? String(toks[0].v).toLowerCase() : '';
    return { type: 'set', name };
  }
  reset() {
    if (this.acceptKw('ROLE')) return { type: 'setRole', role: null };
    const toks = this.rest();
    const name = toks.length >= 3 && toks[1].t === 'op' && toks[1].v === '.' ? (String(toks[0].v) + '.' + String(toks[2].v)).toLowerCase() : toks.length ? String(toks[0].v).toLowerCase() : '';
    return { type: 'set', name, reset: true };
  }
  drop() {
    const kind = this.ident().toUpperCase(); let ifExists = false;
    if (this.acceptKw('IF')) { this.expectKw('EXISTS'); ifExists = true; }
    const name = this.qualifiedName(); if (this.isOp('(')) this.skipParens();
    this.acceptKw('CASCADE') || this.acceptKw('RESTRICT');
    return { type: 'drop', kind, ifExists, name };
  }
}

function tokText(toks) {
  let out = '', prev = null;
  for (const t of toks) {
    const word = t.t === 'id' || t.t === 'num' || t.t === 'str' || t.t === 'param';
    if (prev && word && prev) out += ' ';
    out += t.t === 'str' ? `'${t.v}'` : t.t === 'param' ? '$' + t.v : t.v;
    prev = word;
  }
  return out;
}

function splitStatements(src) {
  // [{text, start, end, toks}], using the tokenizer so that a ; inside a string or a $$ body is not a boundary.
  const toks = tokenize(src);
  const out = []; let cur = []; let start = null;
  const flush = (end) => { if (cur.length) out.push({ start, end, toks: cur }); cur = []; start = null; };
  for (const t of toks) {
    if (t.t === 'meta') { flush(t.s); out.push({ start: t.s, end: t.e, toks: [t] }); continue; }
    if (t.t === 'op' && t.v === ';') { flush(t.s); continue; }
    if (start === null) start = t.s;
    cur.push(t);
  }
  flush(src.length);
  for (const s of out) s.text = src.slice(s.start, s.toks[s.toks.length - 1].e);
  return out;
}

/* The first words of a statement, as the engine's dispatcher sees them (upper case, punctuation kept). */
function leadingWords(toks, n) {
  return toks.slice(0, n).map((t) => (t.t === 'id' ? t.v.toUpperCase() : t.t === 'op' ? t.v : String(t.v).toUpperCase()));
}

/* ------------------------------------------------------------------ helpers */

const qi = (s) => '"' + String(s).replace(/"/g, '""') + '"';
const ql = (v) => (v === null || v === undefined ? 'NULL' : typeof v === 'number' ? String(v) : typeof v === 'boolean' ? (v ? '1' : '0') : "'" + String(v).replace(/'/g, "''") + "'");
const CT = '__ct';
const ADMIN = 'admin';
const PUBLIC = 'public';
const KEYSEP = '\u0000';

function typeKind(text) {
  const base = String(text).trim().toUpperCase().split('(')[0].trim();
  if (['TEXT', 'VARCHAR', 'CHAR', 'CHARACTER VARYING', 'CHARACTER', 'STRING'].includes(base)) return 'text';
  if (['BIGINT', 'INT8', 'INT', 'INTEGER', 'INT4', 'SMALLINT', 'INT2'].includes(base)) return 'int';
  if (['BOOLEAN', 'BOOL'].includes(base)) return 'bool';
  if (['DOUBLE', 'DOUBLE PRECISION', 'FLOAT', 'FLOAT8', 'REAL', 'FLOAT4', 'NUMERIC', 'DECIMAL'].includes(base)) return 'float';
  if (base === 'DATE') return 'date';
  if (['TIMESTAMP', 'TIMESTAMP WITHOUT TIME ZONE', 'TIMESTAMPTZ', 'TIMESTAMP WITH TIME ZONE'].includes(base)) return 'timestamp';
  return null;
}
function typeOrError(text) { const k = typeKind(text); if (!k) throw sqlerr('42704', `type "${String(text).trim().toLowerCase()}" is not supported`); return k; }
function sqlTypeName(text) {
  const base = String(text).trim().toUpperCase().split('(')[0].trim();
  const k = typeKind(text);
  if (k === 'int') return ['BIGINT', 'INT8'].includes(base) ? 'BIGINT' : ['SMALLINT', 'INT2'].includes(base) ? 'SMALLINT' : 'INTEGER';
  return { text: 'TEXT', float: 'DOUBLE PRECISION', bool: 'BOOLEAN', date: 'DATE', timestamp: 'TIMESTAMP' }[k] || base;
}
const SQLITE_TYPE = { text: 'TEXT', int: 'INTEGER', float: 'REAL', bool: 'INTEGER', date: 'TEXT', timestamp: 'TEXT' };
function castTo(sql, kind) {
  switch (kind) {
    case 'int': return `CAST(${sql} AS INTEGER)`;
    case 'float': return `CAST(${sql} AS REAL)`;
    case 'bool': return `(CASE WHEN ${sql} IS NULL THEN NULL WHEN lower(CAST(${sql} AS TEXT)) IN ('1','t','true','yes','on','y') THEN 1 ELSE 0 END)`;
    case 'date': return `date(${sql})`;
    case 'timestamp': return `datetime(${sql})`;
    default: return `CAST(${sql} AS TEXT)`;
  }
}
const textOf = (sql) => `CAST(${sql} AS TEXT)`;

function unitSecs(unit) {
  const u = String(unit).toLowerCase().replace(/s$/, '');
  return { second: 1, sec: 1, minute: 60, min: 60, hour: 3600, day: 86400 }[u] || null;
}
function parseLimit(s) {
  if (!s) return null;
  const m = /^\s*(\d+)\s*\/\s*(\w+)\s*$/.exec(String(s)); if (!m) return null;
  const per = unitSecs(m[2]); return per ? { kind: 'limit', count: Number(m[1]), per } : null;
}
function describeLimit(l) {
  const unit = { 1: 'SECOND', 60: 'MINUTE', 3600: 'HOUR', 86400: 'DAY' }[l.per] || `${l.per} SECONDS`;
  return `${l.count} PER ${unit}`;
}
function intervalMods(text) {
  const parts = String(text).trim().toLowerCase().split(/\s+/); const mods = [];
  for (let i = 0; i < parts.length; i += 2) {
    const n = Number(parts[i]); let u = (parts[i + 1] || 'day').replace(/s$/, '');
    if (!Number.isFinite(n)) throw sqlerr('22007', `invalid interval "${text}"`);
    let k = n; if (u === 'week') { u = 'day'; k = n * 7; }
    if (!['year', 'month', 'day', 'hour', 'minute', 'second'].includes(u)) throw sqlerr('22007', `invalid interval "${text}"`);
    mods.push({ n: k, u });
  }
  return mods;
}
function cmpPair(a, b) { return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : a[1] < b[1] ? -1 : a[1] > b[1] ? 1 : 0; }
const sorted = (it) => [...it].sort();

class UF {
  constructor() { this.p = new Map(); }
  find(x) { if (!this.p.has(x)) this.p.set(x, x); let r = x; while (this.p.get(r) !== r) r = this.p.get(r); this.p.set(x, r); return r; }
  union(a, b) { const ra = this.find(a), rb = this.find(b); if (ra !== rb) this.p.set(ra, rb); }
}

/* ------------------------------------------------------------------ the catalog */

function newCatalog() {
  return {
    tables: new Map(), functions: new Map(), functionOrder: [],
    endpoints: new Map([['analytics', 'MASKED_READ'], ['operational', 'OPERATIONAL']]),
    roles: new Map([[ADMIN, { name: ADMIN, login: true, superuser: true, password: null, matchLimit: { kind: 'default' } }]]),
    tablePrivs: new Map(), domainPrivs: new Map(), endpointGrants: new Map(), fnExecute: new Map()
  };
}
const cloneCatalog = (c) => (typeof structuredClone === 'function' ? structuredClone(c) : deepClone(c));
function deepClone(v) {
  if (v instanceof Map) return new Map([...v].map(([k, x]) => [k, deepClone(x)]));
  if (v instanceof Set) return new Set([...v].map(deepClone));
  if (Array.isArray(v)) return v.map(deepClone);
  if (v && typeof v === 'object') { const o = {}; for (const k of Object.keys(v)) o[k] = deepClone(v[k]); return o; }
  return v;
}
const C = {
  isSuper: (cat, r) => !!(cat.roles.get(r) || {}).superuser,
  privs: (cat, role, t) => [role, PUBLIC].map((r) => cat.tablePrivs.get(r + KEYSEP + t)).filter(Boolean),
  canSelect: (cat, role, t, c) => C.isSuper(cat, role) || C.privs(cat, role, t).some((p) => p.selectAll || p.selectCols.has(c)),
  canSelectAny: (cat, role, t) => C.isSuper(cat, role) || C.privs(cat, role, t).some((p) => p.selectAll || p.selectCols.size > 0),
  canInsert: (cat, role, t) => C.isSuper(cat, role) || C.privs(cat, role, t).some((p) => p.insert),
  canUpdate: (cat, role, t, c) => C.isSuper(cat, role) || C.privs(cat, role, t).some((p) => p.updateAll || p.updateCols.has(c)),
  canDelete: (cat, role, t) => C.isSuper(cat, role) || C.privs(cat, role, t).some((p) => p.delete),
  hasDomainPriv: (cat, role, d, kind) => C.isSuper(cat, role) || [role, PUBLIC].some((r) => { const e = cat.domainPrivs.get(r + KEYSEP + d); return !!(e && e[kind]); }),
  canConnect: (cat, role, ep) => { if (C.isSuper(cat, role)) return true; const s = cat.endpointGrants.get(ep); return !!(s && (s.has(role) || s.has(PUBLIC))); },
  canExecute: (cat, role, f) => { if (C.isSuper(cat, role) || f.owner === role) return true; const s = cat.fnExecute.get(f.name); return !!(s && (s.has(role) || s.has(PUBLIC))); },
  domains: (cat) => { const s = new Set(); for (const t of cat.tables.values()) for (const c of t.columns) if (c.domain) s.add(c.domain); return s; },
  referencesTo: (cat, table) => { const v = []; for (const t of cat.tables.values()) for (const c of t.columns) if (c.references && c.references.table === table) v.push([t.name, c.name, c.references.col]); return v; },
  indexOwner: (cat, name) => { for (const t of cat.tables.values()) if (t.indexes.some((i) => i.name === name)) return t; return null; },
  singleIndex: (t, col) => t.indexes.find((i) => i.columns.length === 1 && i.columns[0] === col),
  column: (t, name) => t.columns.find((c) => c.name === name)
};
function ensureIndex(t, col) {
  if (C.singleIndex(t, col)) return;
  let name = `${t.name}_${col}_idx`, n = 1;
  while (t.indexes.some((i) => i.name === name)) { n++; name = `${t.name}_${col}_idx${n}`; }
  t.indexes.push({ name, columns: [col], unique: false, primary: false, auto: true, enforced: true });
}
function deriveIndexes(t) {
  const pk = t.columns.filter((c) => c.pk).map((c) => c.name);
  if (pk.length && !t.indexes.some((i) => i.primary)) t.indexes.unshift({ name: `${t.name}_pkey`, columns: pk, unique: true, primary: true, auto: true, enforced: true });
  for (const c of t.columns) if (c.domain || c.references) ensureIndex(t, c.name);
}
function physColumns(t) { const out = []; for (const c of t.columns) { out.push(c.name); if (c.domain) out.push(c.name + CT); } return out; }

const PLACE = { filter: 'a filter', join: 'a join condition', project: 'an expression', group: 'GROUP BY', aggregate: 'an aggregate', sort: 'ORDER BY', window: 'a window function' };
const CONTROL = new Set(['BEGIN', 'START', 'COMMIT', 'END', 'ROLLBACK', 'ABORT', 'SAVEPOINT', 'RELEASE']);
const SCALARS = {
  lower: 1, upper: 1, length: 1, char_length: 1, character_length: 1, octet_length: 1, abs: 1, round: 1, ceil: 1, ceiling: 1, floor: 1, sqrt: 1, power: 1, mod: 1,
  coalesce: 1, nullif: 1, substr: 1, substring: 1, trim: 1, ltrim: 1, rtrim: 1, btrim: 1, replace: 1, concat: 1, greatest: 1, least: 1, date_trunc: 1, date_part: 1,
  left: 1, right: 1, strpos: 1, starts_with: 1, now: 1, current_date: 1, reverse: 1, lpad: 1, rpad: 1, to_date: 1, to_timestamp: 1, md5: 1, sha256: 1, random: 1
};

/* ------------------------------------------------------------------ the server */

class Engine {
  constructor(SQL, opts) {
    this.SQL = SQL; this.opts = opts || {};
    this.defaultLimit = parseLimit(this.opts.matchLimit);
    this.reset();
  }

  reset() {
    if (this.db) try { this.db.close(); } catch (e) { /* closed */ }
    if (this.snapshot) try { this.snapshot.close(); } catch (e) { /* closed */ }
    this.db = new this.SQL.Database(); this.prepareDb(this.db);
    this.snapshot = null; this.lockHolder = null;
    this.keys = new Keyring();
    this.committed = newCatalog();
    this.nextTid = 1; this.commitSeq = 0; this.tableSeq = new Map();
    this.audit = []; this.auditSeq = 0; this.windows = new Map(); this.sessionSeq = 0;
    this.s = null;
    this.setupResults = [];
    const setup = typeof this.opts.setup === 'function' ? this.opts.setup() : this.opts.setup;
    if (setup) {
      const s = this.openSession(ADMIN, 'operational');
      this.setupResults = this.runScript(s, setup, []);
      this.closeSession(s);
      const bad = this.setupResults.filter((r) => !r.ok);
      if (bad.length && typeof console !== 'undefined') console.error('setup failed', bad.map((b) => `${b.error.sqlstate} ${b.error.message} :: ${b.text}`));
    }
  }

  prepareDb(db) {
    const self = this;
    db.run('PRAGMA case_sensitive_like = ON');
    db.create_function('dep_index', (d, v) => (v === null || v === undefined ? null : self.keys.indexEntry(String(d), String(v))));
    db.create_function('dep_encrypt', (d, v) => (v === null || v === undefined ? null : self.keys.encrypt(String(d), String(v))));
    db.create_function('dep_decrypt', (d, ct) => (ct === null || ct === undefined ? null : self.keys.decrypt(String(d), String(ct))));
    db.create_function('dep_date_trunc', (unit, v) => {
      if (v === null || v === undefined) return null;
      const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/.exec(String(v)); if (!m) return null;
      const u = String(unit).toLowerCase(); const [, Y, Mo, D, h = '00', mi = '00', s = '00'] = m;
      if (u === 'year') return `${Y}-01-01 00:00:00`;
      if (u === 'quarter') return `${Y}-${String(Math.floor((Number(Mo) - 1) / 3) * 3 + 1).padStart(2, '0')}-01 00:00:00`;
      if (u === 'month') return `${Y}-${Mo}-01 00:00:00`;
      if (u === 'week') { const d = new Date(Date.UTC(+Y, +Mo - 1, +D)); d.setUTCDate(d.getUTCDate() - ((d.getUTCDay() + 6) % 7)); return d.toISOString().slice(0, 10) + ' 00:00:00'; }
      if (u === 'day') return `${Y}-${Mo}-${D} 00:00:00`;
      if (u === 'hour') return `${Y}-${Mo}-${D} ${h}:00:00`;
      if (u === 'minute') return `${Y}-${Mo}-${D} ${h}:${mi}:00`;
      return `${Y}-${Mo}-${D} ${h}:${mi}:${s}`;
    });
    db.create_function('dep_date_part', (field, v) => {
      if (v === null || v === undefined) return null;
      const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/.exec(String(v)); if (!m) return null;
      const [, Y, Mo, D, h = '0', mi = '0', s = '0'] = m; const d = new Date(Date.UTC(+Y, +Mo - 1, +D));
      switch (String(field).toLowerCase()) {
        case 'year': return +Y; case 'quarter': return Math.floor((+Mo - 1) / 3) + 1; case 'month': return +Mo; case 'day': return +D;
        case 'hour': return +h; case 'minute': return +mi; case 'second': return +s;
        case 'dow': return d.getUTCDay(); case 'doy': return Math.round((d - Date.UTC(+Y, 0, 1)) / 86400000) + 1;
        case 'week': { const t = new Date(d); t.setUTCDate(t.getUTCDate() + 3 - ((t.getUTCDay() + 6) % 7)); const w1 = new Date(Date.UTC(t.getUTCFullYear(), 0, 4)); return 1 + Math.round(((t - w1) / 86400000 - 3 + ((w1.getUTCDay() + 6) % 7)) / 7); }
        case 'epoch': return Date.UTC(+Y, +Mo - 1, +D, +h, +mi, +s) / 1000;
        default: return null;
      }
    });
    db.create_function('dep_sha256', (v) => (v === null || v === undefined ? null : hex(sha256(enc(String(v))))));
  }

  /* --- sessions */
  openSession(user, endpoint) {
    const cat = this.committed;
    const ep = String(endpoint || 'operational').toLowerCase(); user = String(user).toLowerCase();
    const path = cat.endpoints.get(ep);
    if (!path) throw sqlerr('08001', `endpoint "${ep}" does not exist`, 'The endpoints are operational (OPERATIONAL) and analytics (MASKED_READ), plus any made with CREATE ENDPOINT.');
    const role = cat.roles.get(user);
    if (!role) throw sqlerr('28000', `role "${user}" does not exist`);
    if (!role.login) throw sqlerr('28000', `role "${user}" is not permitted to log in`);
    if (!C.canConnect(cat, user, ep)) throw sqlerr('28000', `role "${user}" is not allowed to connect through endpoint "${ep}"`, `A superuser can allow it: GRANT CONNECT ON ENDPOINT ${ep} TO ${user}.`);
    return { id: ++this.sessionSeq, sessionUser: user, role: user, endpoint: ep, path, txn: null, notes: [], pendingAudit: [], lastNotes: [] };
  }
  closeSession(s) { if (s && s.txn) { this.s = s; this.rollbackTxn(s); } }
  sessionInfo(s) { return { role: s.role, sessionUser: s.sessionUser, endpoint: s.endpoint, path: s.path, inTransaction: !!(s.txn && s.txn.explicit), failed: !!(s.txn && s.txn.failed) }; }

  cat() { const s = this.s; return (s && s.txn && s.txn.catalog) || this.committed; }
  role() { return this.s.role; }
  catMut(fn) {
    const t = this.s.txn;
    if (!t.stmtCat) { t.prevCat = t.catalog; t.catalog = cloneCatalog(t.catalog || this.committed); t.stmtCat = true; }
    return fn(t.catalog);
  }
  readDb() { return this.lockHolder && this.lockHolder !== this.s && this.snapshot ? this.snapshot : this.db; }
  requireSuper(what) { if (!C.isSuper(this.cat(), this.role())) throw superOnly(what); }
  requireOperational(what) { if (this.s.path === 'MASKED_READ') throw masked(what); }
  addNotes(list) { for (const n of list) if (!this.s.notes.includes(n)) this.s.notes.push(n); }

  /* --- transactions (the DEP Transaction Manager, simulated on one SQLite database) */
  inTxn(s) { return !!(s.txn && s.txn.explicit); }
  beginTxn(s, explicit) { s.txn = { explicit, failed: false, beginSeq: this.commitSeq, catalog: null, writer: false, written: new Set(), savepoints: [], stmtSp: false, stmtCat: false, prevCat: null }; }
  failTxn(s) { if (s.txn && s.txn.explicit) s.txn.failed = true; }
  ensureWriter() {
    const s = this.s, t = s.txn;
    if (!t.writer) {
      if (this.lockHolder && this.lockHolder !== s) throw sqlerr('55P03', 'could not obtain the write lock within 0 s: another transaction is writing', 'Commit or roll back the other transaction, or retry.');
      if (t.explicit) {
        // Other connections go on reading the last commit while this transaction writes.
        const bytes = this.db.export(); this.prepareDb(this.db);
        this.snapshot = new this.SQL.Database(bytes); this.prepareDb(this.snapshot);
      } else t.beginSeq = this.commitSeq;
      this.db.run('BEGIN');
      this.lockHolder = s; t.writer = true;
    }
    if (!t.stmtSp) { this.db.run('SAVEPOINT dep_stmt'); t.stmtSp = true; }
  }
  writeTable(name) {
    this.ensureWriter();
    const t = this.s.txn;
    const meta = this.cat().tables.get(name);
    if (!meta) throw sqlerr('42P01', `relation "${name}" does not exist`);
    if (t.explicit && !t.written.has(meta.tid) && (this.tableSeq.get(meta.tid) || 0) > t.beginSeq) {
      throw sqlerr('40001', `could not serialize access due to a concurrent update of ${name}`, 'Another transaction committed a change to this table after this transaction began. Roll back and run it again.');
    }
    t.written.add(meta.tid);
    return meta;
  }
  releaseLock(s) {
    if (this.lockHolder === s) { this.lockHolder = null; if (this.snapshot) { try { this.snapshot.close(); } catch (e) { /* closed */ } this.snapshot = null; } }
  }
  commitTxn(s) {
    const t = s.txn; s.txn = null;
    if (!t || !t.writer) return;
    this.db.run('COMMIT');
    if (t.catalog) this.committed = t.catalog;
    this.commitSeq++;
    for (const tid of t.written) this.tableSeq.set(tid, this.commitSeq);
    this.releaseLock(s);
  }
  rollbackTxn(s) {
    const t = s.txn; s.txn = null;
    if (t && t.writer) { try { this.db.run('ROLLBACK'); } catch (e) { /* nothing open */ } this.releaseLock(s); }
  }
  stmtEnd(ok) {
    const t = this.s.txn; if (!t) return;
    if (t.stmtSp) { if (ok) this.db.run('RELEASE dep_stmt'); else { this.db.run('ROLLBACK TO dep_stmt'); this.db.run('RELEASE dep_stmt'); } }
    if (!ok && t.stmtCat) t.catalog = t.prevCat;
    t.stmtSp = false; t.stmtCat = false; t.prevCat = null;
  }

  /* --- match limits and the audit log */
  matchLimitOf(cat, role) {
    const r = cat.roles.get(role); if (!r) return null;
    if (r.matchLimit.kind === 'limit') return r.matchLimit;
    if (r.matchLimit.kind === 'none') return null;
    return r.superuser ? null : this.defaultLimit;
  }
  admitEvents(events) {
    if (!events.length) return;
    const role = this.role();
    const matches = events.filter((e) => e.kind === 'match').reduce((s, e) => s + e.count, 0);
    if (matches > 0) {
      const l = this.matchLimitOf(this.cat(), role);
      if (l) {
        const now = Date.now(); const q = (this.windows.get(role) || []).filter(([t]) => now - t < l.per * 1000);
        const used = q.reduce((s, [, c]) => s + c, 0);
        this.windows.set(role, q);
        if (used + matches > l.count) {
          this.s.pendingAudit.push({ kind: 'rate_limited', object: events.filter((e) => e.kind === 'match').map((e) => e.object).join(', '), domain: null, count: matches });
          throw dep('DEP_MATCH_RATE_LIMITED', `Role ${role} has made ${used} protected matches of the ${describeLimit(l)} it is allowed; this statement would make ${matches} more.`, 'Wait for the period to pass, or ask a superuser to change the limit (ALTER ROLE ... MATCH LIMIT n PER MINUTE).');
        }
        q.push([now, matches]);
      }
    }
    this.s.pendingAudit.push(...events);
  }
  flushAudit(s, err, first) {
    const events = s.pendingAudit; s.pendingAudit = [];
    const mk = (kind, object, domain, count, detail) => ({ seq: ++this.auditSeq, at: new Date().toISOString(), session_user: s.sessionUser, role: s.role, endpoint: s.endpoint, path: s.path, kind, object, domain: domain || null, count: count || 0, detail: detail || null });
    for (const e of events) this.audit.push(mk(e.kind, e.object, e.domain, e.count, err && e.kind !== 'rate_limited' ? 'statement failed' : null));
    if (err && err.code && err.code !== 'DEP_MATCH_RATE_LIMITED') this.audit.push(mk('denied', first, null, 0, err.code));
  }

  /* --- running statements */
  runScript(s, src, params) {
    params = (params || []).map((v) => (v === undefined ? null : v));
    let stmts;
    try { stmts = splitStatements(src); } catch (e) { return [{ ok: false, text: String(src).trim(), notes: [], error: errObj(e), session: this.sessionInfo(s) }]; }
    const out = [];
    for (const st of stmts) { const r = this.runOne(s, st, params); out.push(r); if (r.newSession) s = r.newSession; }
    return out;
  }

  runOne(s, st, params) {
    const out = { ok: true, text: st.text, notes: [], compiled: [] };
    this.s = s;
    const toks = st.toks;
    if (toks[0].t === 'meta') {
      try { Object.assign(out, this.meta(s, toks[0].v)); } catch (e) { out.ok = false; out.error = errObj(e); }
      out.session = this.sessionInfo(out.newSession || s);
      return out;
    }
    const words = leadingWords(toks, 4);
    const first = words[0] || '';
    if (!(first === 'SHOW' && words[1] === 'DEP' && words[2] === 'NOTES')) { s.lastNotes = s.notes; s.notes = []; }
    s.pendingAudit = [];
    const ctl = CONTROL.has(first);
    let err = null, res = null;
    if (s.txn && s.txn.failed && !['COMMIT', 'END', 'ROLLBACK', 'ABORT'].includes(first)) {
      err = sqlerr('25P02', 'current transaction is aborted, commands ignored until end of transaction block', 'ROLLBACK (or ROLLBACK TO a savepoint), then run the statements again.');
    } else {
      const implicit = !ctl && !this.inTxn(s);
      if (implicit) this.beginTxn(s, false);
      try {
        res = this.execute(s, toks, words, params, out);
        if (!ctl) this.stmtEnd(true);
      } catch (e) {
        err = toSqlError(e);
        if (!ctl && s.txn) { try { this.stmtEnd(false); } catch (x) { /* already undone */ } }
      }
      if (implicit) {
        if (!err) { try { this.commitTxn(s); } catch (e) { err = toSqlError(e); this.rollbackTxn(s); } }
        else this.rollbackTxn(s);
      }
      this.flushAudit(s, err, first);
      if (err) {
        const ends = ['COMMIT', 'END', 'ABORT'].includes(first) || (first === 'ROLLBACK' && words[1] !== 'TO');
        if (!ends) this.failTxn(s);
      }
    }
    if (err) { out.ok = false; out.error = errObj(err); } else Object.assign(out, res);
    out.notes = s.notes.slice();
    out.session = this.sessionInfo(s);
    return out;
  }

  meta(s, text) {
    const parts = text.slice(1).trim().split(/\s+/);
    const cmd = parts[0];
    if (cmd === 'connect' || cmd === 'c') {
      const ns = this.openSession(parts[1] || s.sessionUser, parts[2] || s.endpoint);
      this.closeSession(s);
      return { kind: 'command', tag: `You are now connected as "${ns.role}" through endpoint "${ns.endpoint}" (${ns.path}).`, newSession: ns };
    }
    throw sqlerr('42601', `invalid command \\${cmd}`, 'The simulator supports \\connect role [endpoint].');
  }

  execute(s, toks, words, params, out) {
    const first = words[0];
    let ast = null, parseErr = null;
    try { const p = new Parser(toks); ast = p.statement(); if (!p.eof()) p.fail(); } catch (e) { parseErr = e; }
    const ddl = ['CREATE', 'DROP', 'GRANT', 'REVOKE', 'COMMENT', 'REINDEX'].includes(first) || (first === 'ALTER' && words[1] !== 'SYSTEM');
    if (ddl) { this.requireOperational('Schema changes are'); this.ensureWriter(); }
    if (['INSERT', 'UPDATE', 'DELETE', 'MERGE', 'TRUNCATE'].includes(first)) this.requireOperational('Writes are');
    if (parseErr) throw parseErr;
    const ctx = this.newCtx({ params, compiled: out.compiled });
    switch (ast.type) {
      case 'select': case 'compound': case 'values': return this.execQuery(ast, ctx);
      case 'explain': return this.execExplain(ast, ctx);
      case 'insert': return this.execInsert(ast, ctx);
      case 'update': return this.execUpdate(ast, ctx);
      case 'delete': return this.execDelete(ast, ctx);
      case 'createTable': this.requireSuper('create tables'); return this.execCreateTable(ast, ctx);
      case 'createIndex': this.requireSuper('create indexes'); return this.execCreateIndex(ast, ctx);
      case 'reindex': this.requireSuper('rebuild indexes'); return this.execReindex(ast);
      case 'createEndpoint': this.requireSuper('create endpoints'); return this.execCreateEndpoint(ast);
      case 'createFunction': return this.execCreateFunction(ast, ctx);
      case 'createOther': case 'copy': this.requireOperational('Schema changes are'); throw unsupported('Only CREATE TABLE, CREATE INDEX, CREATE FUNCTION and CREATE ENDPOINT are supported.');
      case 'alterOther': throw unsupported('Only ALTER ROLE and ALTER SYSTEM ROTATE MASTER KEY are supported.');
      case 'otherStmt': throw unsupported(`Statement not supported: ${ast.word}`);
      case 'rotateKey': return this.execRotateKey(ctx);
      case 'role': return this.execRole(ast);
      case 'grant': return this.execGrant(ast);
      case 'comment': return this.execComment(ast);
      case 'drop': return this.execDrop(ast, ctx);
      case 'setRole': return this.execSetRole(ast);
      case 'set':
        if (/^dep\./.test(ast.name)) {
          if (ast.name === 'dep.path') throw dep('DEP_PATH_IMMUTABLE', `The access path of this connection is ${s.path} and cannot be changed.`, 'The path is fixed at authentication. Connect through another endpoint instead.');
          throw sqlerr('42704', `unrecognized configuration parameter "${ast.name}"`);
        }
        return { kind: 'command', tag: 'SET' };
      case 'show': return this.execShow(ast, s);
      case 'begin':
        if (this.inTxn(s)) { this.addNotes(['There is already a transaction in progress.']); return { kind: 'command', tag: 'BEGIN' }; }
        this.beginTxn(s, true); return { kind: 'command', tag: 'BEGIN' };
      case 'commit':
        if (!s.txn) { this.addNotes(['There is no transaction in progress.']); return { kind: 'command', tag: 'COMMIT' }; }
        if (s.txn.failed) { this.rollbackTxn(s); this.addNotes(['The transaction had failed, so it was rolled back.']); return { kind: 'command', tag: 'ROLLBACK' }; }
        this.commitTxn(s); return { kind: 'command', tag: 'COMMIT' };
      case 'rollback':
        if (!s.txn) this.addNotes(['There is no transaction in progress.']); else this.rollbackTxn(s);
        return { kind: 'command', tag: 'ROLLBACK' };
      case 'savepoint': return this.execSavepoint(s, ast.name);
      case 'rollbackTo': return this.execRollbackTo(s, ast.name);
      case 'release': return this.execRelease(s, ast.name);
      default: throw unsupported('This statement is not supported.');
    }
  }

  execSavepoint(s, name) {
    const t = s.txn; if (!t || !t.explicit) throw sqlerr('25P01', 'SAVEPOINT can only be used in transaction blocks');
    let sq = null;
    if (t.writer) { sq = 'dep_sp' + (t.savepoints.length + 1) + '_' + (++this.auditSeq); this.db.run(`SAVEPOINT ${qi(sq)}`); }
    t.savepoints.push({ name, sq, catalog: t.catalog ? cloneCatalog(t.catalog) : null });
    return { kind: 'command', tag: 'SAVEPOINT' };
  }
  execRollbackTo(s, name) {
    const t = s.txn; if (!t || !t.explicit) throw sqlerr('25P01', 'ROLLBACK TO SAVEPOINT can only be used in transaction blocks');
    let at = -1; for (let i = t.savepoints.length - 1; i >= 0; i--) if (t.savepoints[i].name === name) { at = i; break; }
    if (at < 0) throw sqlerr('3B001', `savepoint "${name}" does not exist`);
    t.savepoints.length = at + 1;
    const sp = t.savepoints[at];
    if (t.writer) {
      if (sp.sq) this.db.run(`ROLLBACK TO ${qi(sp.sq)}`);
      else { this.db.run('ROLLBACK'); this.db.run('BEGIN'); }
    }
    t.catalog = sp.catalog ? cloneCatalog(sp.catalog) : null;
    t.failed = false;
    return { kind: 'command', tag: 'ROLLBACK' };
  }
  execRelease(s, name) {
    const t = s.txn; if (!t || !t.explicit) throw sqlerr('25P01', 'RELEASE SAVEPOINT can only be used in transaction blocks');
    let at = -1; for (let i = t.savepoints.length - 1; i >= 0; i--) if (t.savepoints[i].name === name) { at = i; break; }
    if (at < 0) throw sqlerr('3B001', `savepoint "${name}" does not exist`);
    const gone = t.savepoints.splice(at);
    const firstNamed = gone.find((x) => x.sq);
    if (t.writer && firstNamed) this.db.run(`RELEASE ${qi(firstNamed.sq)}`);
    return { kind: 'command', tag: 'RELEASE' };
  }

  /* ================================================================ the analyzer */

  newCtx(o) {
    return Object.assign({ role: this.s.role, path: this.s.path, region: null, notes: [], events: [], uses: { columns: new Set(), tables: new Set(), functions: new Set() }, ctes: [], scans: [], params: [], compiled: [], seq: { n: 0 } }, o || {});
  }
  note(ctx, text) { if (!ctx.region && !ctx.notes.includes(text)) ctx.notes.push(text); }
  event(ctx, kind, object, domain, count) { if (!ctx.region) ctx.events.push({ kind, object, domain, count }); }
  links(ctx, d) { return !!(ctx.region && ctx.region.links.has(d)); }
  checkMatchAllowed(ctx) { if (!ctx.region && ctx.path === 'MASKED_READ') throw masked('Matching PROTECTED values is'); }

  newBlock(parent) { return { rels: [], parent: parent || null, edges: [], anchors: new Set(), using: new Set(), lookups: [], equivs: [] }; }

  /* Every join between protected columns must be connected to an anchor (or run in a LINKS region). */
  closeBlock(b, ctx) {
    if (!b.edges.length) return;
    const uf = new UF(); for (const e of b.edges) uf.union(e.a, e.b);
    const roots = new Set([...b.anchors].map((a) => uf.find(a)));
    for (const e of b.edges) {
      const ok = roots.has(uf.find(e.a));
      if (ok) this.note(ctx, `Join of PROTECTED columns ${e.da} and ${e.db} (MATCH DOMAIN ${e.domain}) is anchored to a single protected value.`);
      if (!ok && !this.links(ctx, e.domain)) {
        const sub = e.db === 'a subquery';
        const x = sub ? e.da : e.db, y = sub ? e.db : e.da;
        throw dep('DEP_UNANCHORED_MATCH',
          sub ? `Match of PROTECTED column ${x} against ${y} is not anchored to a single protected value.` : `Join on PROTECTED columns ${x} and ${y} is not anchored to a single protected value.`,
          `Anchor the join with a lookup such as WHERE ${x} = $1, or perform it inside a function declared LINKS ${e.domain}.`);
      }
    }
  }
  isAnchored(b, key) {
    const uf = new UF(); for (const e of b.edges) uf.union(e.a, e.b);
    const roots = new Set([...b.anchors].map((a) => uf.find(a)));
    return roots.has(uf.find(key));
  }

  resolve(b, ctx, table, name) {
    if (name.endsWith(CT)) throw sqlerr('42703', `column "${name}" does not exist`);
    if (!table && ctx.region && ctx.region.params.has(name)) return { param: ctx.region.params.get(name) };
    let scope = b, depth = 0;
    while (scope) {
      const hits = [];
      for (const r of scope.rels) {
        if (table && r.alias !== table) continue;
        const c = r.cols.get(name);
        if (c) hits.push({ rel: r, col: c });
      }
      if (hits.length > 1 && !(scope.using.has(name))) throw sqlerr('42702', `column reference "${name}" is ambiguous`);
      if (hits.length) return { rel: hits[0].rel, col: hits[0].col, depth };
      scope = scope.parent; depth++;
    }
    const fields = [];
    for (const r of b.rels) {
      if (r.kind === 'table') for (const c of physColumns(r.meta)) fields.push(`${r.display || r.alias}.${c}`);
      else for (const n of r.order) { const c = r.cols.get(n); fields.push(`${r.display || r.alias}.${n}`); if (c.physCt) fields.push(`${r.display || r.alias}.${n}${CT}`); }
    }
    throw sqlerr('42703', `Schema error: No field named ${table ? table + '.' : ''}${name}.\nValid fields are ${fields.join(', ')}.`);
  }

  /* A value: { kind: 'ord' | 'prot' | 'disc', sql, constant, dtype, ... }. */
  ex(e, b, ctx, pos) {
    pos = pos || { place: 'project', conj: false };
    const sub = { place: pos.place, conj: false };
    switch (e.type) {
      case 'lit':
        if (e.ltype === 'str') return { kind: 'ord', sql: ql(e.v), constant: true, value: e.v, dtype: 'text', lit: e };
        if (e.ltype === 'num') return { kind: 'ord', sql: e.raw || String(e.v), constant: true, value: e.v, dtype: 'int', lit: e };
        if (e.ltype === 'float') return { kind: 'ord', sql: e.raw || String(e.v), constant: true, value: e.v, dtype: 'float', lit: e };
        if (e.ltype === 'bool') return { kind: 'ord', sql: e.v ? '1' : '0', constant: true, value: e.v, dtype: 'bool', lit: e };
        return { kind: 'ord', sql: 'NULL', constant: true, value: null, dtype: null, lit: e };
      case 'param': {
        if (e.n < 1 || e.n > ctx.params.length) throw sqlerr('42P02', `there is no parameter $${e.n}`);
        const v = ctx.params[e.n - 1];
        return { kind: 'ord', sql: ql(v), constant: true, value: v, dtype: typeof v === 'number' ? (Number.isInteger(v) ? 'int' : 'float') : typeof v === 'boolean' ? 'bool' : 'text' };
      }
      case 'curdate': return { kind: 'ord', sql: "date('now','localtime')", constant: true, dtype: 'date' };
      case 'curts': return { kind: 'ord', sql: "datetime('now','localtime')", constant: true, dtype: 'timestamp' };
      case 'curuser': return { kind: 'ord', sql: ql(e.which === 'SESSION_USER' ? this.s.sessionUser : ctx.role), constant: true, dtype: 'text' };
      case 'interval': return { kind: 'ord', sql: 'NULL', constant: true, interval: intervalMods(e.text), dtype: 'interval' };
      case 'col': return this.exCol(e, b, ctx, pos);
      case 'star': throw sqlerr('42601', 'syntax error at or near "*"');
      case 'bin': return this.exBin(e, b, ctx, pos);
      case 'not': { const r = this.ex(e.e, b, ctx, sub); return this.generic('NOT', [r], `(NOT ${r.sql})`, { dtype: 'bool' }); }
      case 'istrue': { const r = this.ex(e.e, b, ctx, sub); return this.generic('A boolean test', [r], `((${r.sql}) IS ${e.neg ? 'NOT ' : ''}${e.v ? 'TRUE' : 'FALSE'})`, { dtype: 'bool' }); }
      case 'distinctfrom': { const l = this.ex(e.l, b, ctx, sub), r = this.ex(e.r, b, ctx, sub); return this.generic(`Operator IS ${e.neg ? 'NOT ' : ''}DISTINCT FROM`, [l, r], `(${l.sql} IS ${e.neg ? '' : 'NOT '}${r.sql})`, { dtype: 'bool' }); }
      case 'isnull': {
        const r = this.ex(e.e, b, ctx, sub);
        if (r.kind === 'disc' && pos.place !== 'project') throw disclosedUse(r.desc, PLACE[pos.place]);
        return { kind: 'ord', sql: `(${r.sql} IS ${e.neg ? 'NOT ' : ''}NULL)`, constant: r.kind === 'ord' && r.constant, dtype: 'bool', fromParam: r.fromParam };
      }
      case 'like': {
        const r = this.ex(e.e, b, ctx, sub), p = this.ex(e.pat, b, ctx, sub);
        const sql = e.ci ? `(lower(${r.sql}) ${e.neg ? 'NOT ' : ''}LIKE lower(${p.sql}))` : `(${r.sql} ${e.neg ? 'NOT ' : ''}LIKE ${p.sql})`;
        return this.generic(e.ci ? 'ILIKE' : 'LIKE', [r, p], sql, { dtype: 'bool' });
      }
      case 'between': {
        const r = this.ex(e.e, b, ctx, sub), lo = this.ex(e.lo, b, ctx, sub), hi = this.ex(e.hi, b, ctx, sub);
        return this.generic('BETWEEN', [r, lo, hi], `(${r.sql} ${e.neg ? 'NOT ' : ''}BETWEEN ${lo.sql} AND ${hi.sql})`, { dtype: 'bool' });
      }
      case 'inlist': return this.exInList(e, b, ctx, pos);
      case 'insub': return this.exInSub(e, b, ctx, pos);
      case 'exists': { const q = this.compileQuery(e.q, ctx, { parent: b }); return { kind: 'ord', sql: `(${e.neg ? 'NOT ' : ''}EXISTS (${q.sql}))`, constant: false, dtype: 'bool' }; }
      case 'scalarsub': {
        const q = this.compileQuery(e.q, ctx, { parent: b });
        const oc = q.outCols[0];
        if (!oc) throw sqlerr('42601', 'subquery must return a column');
        const sql = `(SELECT ${qi(oc.name)} FROM (${q.sql}))`;
        if (oc.kind === 'prot') return { kind: 'prot', sql, physCt: null, hasCt: false, domain: oc.domain, desc: oc.desc, vkey: null, anchored: oc.anchored, dtype: 'text' };
        if (oc.kind === 'disc') {
          if (pos.place !== 'project' && pos.place !== 'sort') throw disclosedUse(oc.desc, PLACE[pos.place]);
          return { kind: 'disc', sql, domain: oc.domain, desc: oc.desc, dtype: oc.dtype };
        }
        return { kind: 'ord', sql, constant: false, dtype: oc.dtype };
      }
      case 'cast': {
        const r = this.ex(e.e, b, ctx, sub);
        const k = typeOrError(e.to);
        if (r.kind === 'prot') throw dep('DEP_PROTECTED_OPERATION', `Conversion from PROTECTED to plain ${sqlTypeName(e.to)} is forbidden (${r.desc}).`, 'To release a value, use a disclosing function.');
        const sql = castTo(r.sql, k);
        if (r.kind === 'disc') return { kind: 'disc', sql, domain: r.domain, desc: r.desc, dtype: k };
        return { kind: 'ord', sql, constant: r.constant, value: r.value, dtype: k, fromParam: r.fromParam, castOf: r };
      }
      case 'neg': { const r = this.ex(e.e, b, ctx, sub); return this.generic('Negation', [r], `(-${r.sql})`, { dtype: r.dtype }); }
      case 'arith': return this.exArith(e, b, ctx, pos);
      case 'case': {
        const parts = [], vals = [];
        const base = e.base ? this.ex(e.base, b, ctx, sub) : null; if (base) vals.push(base);
        let dtype = null;
        for (const w of e.whens) { const c = this.ex(w.c, b, ctx, sub), v = this.ex(w.v, b, ctx, sub); vals.push(c, v); dtype = dtype || v.dtype; parts.push(`WHEN ${c.sql} THEN ${v.sql}`); }
        let els = ''; if (e.els) { const v = this.ex(e.els, b, ctx, sub); vals.push(v); dtype = dtype || v.dtype; els = ` ELSE ${v.sql}`; }
        return this.generic('CASE', vals, `(CASE ${base ? base.sql + ' ' : ''}${parts.join(' ')}${els} END)`, { dtype });
      }
      case 'func': return this.exFunc(e, b, ctx, pos);
      default: throw unsupported(`This expression (${e.type}) is not supported.`);
    }
  }

  exCol(e, b, ctx, pos) {
    const r = this.resolve(b, ctx, e.table, e.name);
    if (r.param) { const p = r.param; return { kind: 'ord', sql: castTo(ql(p.value), p.kind), constant: true, value: p.value, dtype: p.kind, fromParam: true }; }
    const c = r.col;
    if (r.depth > 0) {
      if (c.kind === 'prot') throw unsupported('Correlated subqueries on PROTECTED columns are not supported. Write the condition as a join.');
      if (c.kind === 'disc') throw disclosedUse(c.desc, 'a subquery');
    }
    if (c.base) ctx.uses.columns.add(c.base.table + KEYSEP + c.base.col);
    if (c.kind === 'prot') return { kind: 'prot', sql: c.phys, physCt: c.physCt, hasCt: !!c.physCt, domain: c.domain, desc: c.desc, vkey: r.depth ? null : r.rel.alias + '.' + c.name, dtype: 'text', isCol: true, rel: r.rel, colName: c.name, depth: r.depth };
    if (c.kind === 'disc') {
      if (pos.place !== 'project' && pos.place !== 'sort') throw disclosedUse(c.desc, PLACE[pos.place]);
      return { kind: 'disc', sql: c.phys, domain: c.domain, desc: c.desc, dtype: c.dtype, isCol: true };
    }
    return { kind: 'ord', sql: c.phys, constant: false, dtype: c.dtype, isCol: true, rel: r.rel, colName: c.name, depth: r.depth };
  }

  /* An operation that is not equality: a PROTECTED operand is refused; a disclosed one makes the result disclosed. */
  generic(opName, vals, sql, o) {
    o = o || {};
    for (const v of vals) if (v.kind === 'prot') throw protectedOp(opName, v.desc);
    const d = vals.find((v) => v.kind === 'disc');
    if (d && o.agg) throw disclosedUse(d.desc, 'an aggregate');
    if (d) return { kind: 'disc', sql, domain: d.domain, desc: d.desc, dtype: o.dtype };
    return { kind: 'ord', sql, constant: !o.agg && !o.volatile && vals.every((v) => v.constant), dtype: o.dtype, agg: !!o.agg || vals.some((v) => v.agg), fromParam: vals.some((v) => v.fromParam) };
  }

  lookupOf(v, keys) {
    // An equality or IN on a column of a stored table: a candidate index lookup (for EXPLAIN).
    if (!v || !v.isCol || v.depth || !v.rel || v.rel.kind !== 'table') return null;
    return { alias: v.rel.alias, table: v.rel.tableName, col: v.colName, keys };
  }

  equivOf(l, r) { const a = this.lookupOf(l, 0), b = this.lookupOf(r, 0); return a && b ? [a, b] : null; }

  exBin(e, b, ctx, pos) {
    if (e.op === 'AND' || e.op === 'OR') {
      const sp = { place: pos.place, conj: e.op === 'AND' && pos.conj };
      const l = this.ex(e.l, b, ctx, sp), r = this.ex(e.r, b, ctx, sp);
      const out = this.generic(`Operator ${e.op}`, [l, r], `(${l.sql} ${e.op} ${r.sql})`, { dtype: 'bool' });
      if (e.op === 'AND') {
        out.lookups = [].concat(l.lookups || (l.lookup ? [l.lookup] : []), r.lookups || (r.lookup ? [r.lookup] : []));
        out.equivs = [].concat(l.equivs || (l.equiv ? [l.equiv] : []), r.equivs || (r.equiv ? [r.equiv] : []));
      }
      else if (l.lookup && r.lookup && l.lookup.alias === r.lookup.alias && l.lookup.col === r.lookup.col) out.lookup = Object.assign({}, l.lookup, { keys: l.lookup.keys + r.lookup.keys });
      return out;
    }
    const sp = { place: pos.place, conj: false };
    const l = this.ex(e.l, b, ctx, sp), r = this.ex(e.r, b, ctx, sp);
    if (e.op === '=') return this.exEq(l, r, b, ctx, pos);
    if ((l.dtype === 'date' || l.dtype === 'timestamp') && (r.dtype === 'date' || r.dtype === 'timestamp') && l.kind === 'ord' && r.kind === 'ord') {
      const cmp = l.dtype === r.dtype ? null : 'datetime';
      if (cmp) return this.generic(`Operator ${e.op}`, [l, r], `(datetime(${l.sql}) ${e.op} datetime(${r.sql}))`, { dtype: 'bool' });
    }
    return this.generic(`Operator ${e.op === '<>' ? '!=' : e.op}`, [l, r], `(${l.sql} ${e.op} ${r.sql})`, { dtype: 'bool' });
  }

  exEq(l, r, b, ctx, pos) {
    for (const t of [l, r]) if (t.kind === 'disc') throw disclosedUse(t.desc, pos.place === 'project' ? 'a comparison' : PLACE[pos.place]);
    if (l.kind === 'prot' && r.kind === 'prot') {
      if (l.domain !== r.domain) throw domainMismatch(l, r);
      this.checkMatchAllowed(ctx);
      if (!l.vkey || !r.vkey) throw unsupported(l.isCol && r.isCol ? 'This comparison of PROTECTED values is not supported.' : 'Compare PROTECTED columns directly.');
      b.edges.push({ a: l.vkey, b: r.vkey, da: l.desc, db: r.desc, domain: l.domain });
      return { kind: 'ord', sql: `(${l.sql} = ${r.sql})`, constant: false, dtype: 'bool', equiv: this.equivOf(l, r) };
    }
    if (l.kind === 'prot' || r.kind === 'prot') {
      const p = l.kind === 'prot' ? l : r, o = p === l ? r : l;
      if (!o.constant) throw protectedOp('Comparison with a non-PROTECTED column', p.desc);
      this.checkMatchAllowed(ctx);
      if (o.fromParam && ctx.region) ctx.region.paramMatch = true;
      this.note(ctx, `PROTECTED column ${p.desc} is matched by equality through the blind index of MATCH DOMAIN ${p.domain}.`);
      this.event(ctx, 'match', p.desc, p.domain, 1);
      if (pos.conj && p.vkey) b.anchors.add(p.vkey);
      const ie = `dep_index(${ql(p.domain)}, ${textOf(o.sql)})`;
      return { kind: 'ord', sql: p === l ? `(${p.sql} = ${ie})` : `(${ie} = ${p.sql})`, constant: false, dtype: 'bool', lookup: this.lookupOf(p, o.value === null && o.lit ? 0 : 1) };
    }
    let ls = l.sql, rs = r.sql;
    if ((l.dtype === 'date' && r.dtype === 'timestamp') || (l.dtype === 'timestamp' && r.dtype === 'date')) { ls = `datetime(${ls})`; rs = `datetime(${rs})`; }
    const out = { kind: 'ord', sql: `(${ls} = ${rs})`, constant: l.constant && r.constant, dtype: 'bool', fromParam: l.fromParam || r.fromParam };
    out.equiv = this.equivOf(l, r);
    if (l.isCol && r.constant) out.lookup = this.lookupOf(l, r.value === null && r.lit ? 0 : 1);
    else if (r.isCol && l.constant) out.lookup = this.lookupOf(r, l.value === null && l.lit ? 0 : 1);
    return out;
  }

  exInList(e, b, ctx, pos) {
    const sp = { place: pos.place, conj: false };
    const x = this.ex(e.e, b, ctx, sp);
    if (x.kind === 'disc') throw disclosedUse(x.desc, PLACE[pos.place]);
    if (x.kind === 'prot') {
      const items = e.list.map((it) => {
        const v = this.ex(it, b, ctx, sp);
        if (v.kind !== 'ord' || !v.constant) throw protectedOp('IN with a non-constant list', x.desc);
        if (v.fromParam && ctx.region) ctx.region.paramMatch = true;
        return `dep_index(${ql(x.domain)}, ${textOf(v.sql)})`;
      });
      this.checkMatchAllowed(ctx);
      this.note(ctx, `PROTECTED column ${x.desc} is matched against a list of values through the blind index of MATCH DOMAIN ${x.domain}.`);
      this.event(ctx, 'match', x.desc, x.domain, e.list.length);
      if (pos.conj && !e.neg && e.list.length === 1 && x.vkey) b.anchors.add(x.vkey);
      const out = { kind: 'ord', sql: `(${x.sql} ${e.neg ? 'NOT ' : ''}IN (${items.join(', ')}))`, constant: false, dtype: 'bool' };
      if (!e.neg) out.lookup = this.lookupOf(x, e.list.length);
      return out;
    }
    const items = e.list.map((it) => { const v = this.ex(it, b, ctx, sp); if (v.kind === 'prot') throw protectedOp('IN', v.desc); return v; });
    const out = { kind: 'ord', sql: `(${x.sql} ${e.neg ? 'NOT ' : ''}IN (${items.map((v) => v.sql).join(', ')}))`, constant: x.constant && items.every((v) => v.constant), dtype: 'bool' };
    if (!e.neg && x.isCol && items.every((v) => v.constant && v.lit)) out.lookup = this.lookupOf(x, items.filter((v) => v.value !== null).length);
    return out;
  }

  exInSub(e, b, ctx, pos) {
    const x = this.ex(e.e, b, ctx, { place: pos.place, conj: false });
    if (x.kind === 'disc') throw disclosedUse(x.desc, PLACE[pos.place]);
    const q = this.compileQuery(e.q, ctx, { parent: b });
    const oc = q.outCols[0];
    if (!oc) throw sqlerr('42601', 'subquery must return a column');
    const sql = `(${x.sql} ${e.neg ? 'NOT ' : ''}IN (SELECT ${qi(oc.name)} FROM (${q.sql})))`;
    if (x.kind === 'prot' && oc.kind === 'prot') {
      if (x.domain !== oc.domain) throw domainMismatch(x, oc);
      this.checkMatchAllowed(ctx);
      if (!x.vkey) throw unsupported('Match a PROTECTED column directly against a subquery.');
      const syn = '#subquery' + (++ctx.seq.n);
      if (oc.anchored) b.anchors.add(syn);
      b.edges.push({ a: x.vkey, b: syn, da: x.desc, db: 'a subquery', domain: x.domain });
      return { kind: 'ord', sql, constant: false, dtype: 'bool' };
    }
    if (x.kind === 'prot' || oc.kind === 'prot') throw protectedOp('Comparison with a non-PROTECTED column', x.kind === 'prot' ? x.desc : oc.desc);
    if (oc.kind === 'disc') throw disclosedUse(oc.desc, PLACE[pos.place]);
    return { kind: 'ord', sql, constant: false, dtype: 'bool' };
  }

  exArith(e, b, ctx, pos) {
    const sp = { place: pos.place, conj: false };
    const l = this.ex(e.l, b, ctx, sp), r = this.ex(e.r, b, ctx, sp);
    const op = e.op;
    if (l.kind === 'ord' && r.kind === 'ord') {
      if (r.interval && (op === '+' || op === '-') && (l.dtype === 'date' || l.dtype === 'timestamp' || l.dtype === null)) {
        const mods = r.interval.map((m) => ql(`${op === '-' ? -m.n : m.n} ${m.u}`)).join(', ');
        const days = l.dtype === 'date' && r.interval.every((m) => ['year', 'month', 'day'].includes(m.u));
        return { kind: 'ord', sql: `${days ? 'date' : 'datetime'}(${l.sql}, ${mods})`, constant: l.constant, dtype: days ? 'date' : 'timestamp' };
      }
      if (l.dtype === 'date' && (r.dtype === 'int') && (op === '+' || op === '-')) {
        return { kind: 'ord', sql: `date(${l.sql}, printf('%+d day', ${op === '-' ? '-' : ''}(${r.sql})))`, constant: l.constant && r.constant, dtype: 'date', fromParam: l.fromParam || r.fromParam };
      }
      if (l.dtype === 'date' && r.dtype === 'date' && op === '-') {
        return { kind: 'ord', sql: `CAST(julianday(${l.sql}) - julianday(${r.sql}) AS INTEGER)`, constant: l.constant && r.constant, dtype: 'int' };
      }
    }
    if (l.interval || r.interval) throw sqlerr('42883', `operator does not exist: ${op} with an interval here`);
    const dtype = op === '||' ? 'text' : l.dtype === 'float' || r.dtype === 'float' ? 'float' : l.dtype || r.dtype;
    return this.generic(`Operator ${op}`, [l, r], `(${l.sql} ${op} ${r.sql})`, { dtype });
  }

  exFunc(e, b, ctx, pos) {
    const name = e.name.toLowerCase();
    if (name.startsWith('dep_')) throw sqlerr('42883', `function ${e.name}() does not exist`);
    if (name === 'reveal') return this.exReveal(e, b, ctx, pos);
    const isAgg = AGGREGATES.has(name) && !e.over;
    const isWin = !!e.over;
    if (!isAgg && !isWin && !SCALARS[name]) {
      throw sqlerr('XX000', `Error during planning: Invalid function '${name}'.`);
    }
    if (WINDOW_ONLY.has(name) && !isWin) throw sqlerr('42883', `window function ${name}() requires an OVER clause`);
    const argPlace = isAgg ? 'aggregate' : isWin ? 'window' : pos.place;
    const ap = { place: argPlace, conj: false };
    const args = e.args.map((a) => this.ex(a, b, ctx, ap));
    // COUNT(x) counts rows; it matches nothing.
    if (name === 'count' && !e.distinct && !isWin) {
      for (const a of args) if (a.kind === 'disc') throw disclosedUse(a.desc, 'an aggregate');
      return { kind: 'ord', sql: e.star ? 'count(*)' : `count(${args.map((a) => a.sql).join(', ')})`, constant: false, dtype: 'int', agg: true };
    }
    if (e.star && name !== 'count') throw sqlerr('42601', `${name}(*) is not valid`);
    let overSql = '';
    const extra = [];
    if (isWin) {
      const wp = { place: 'window', conj: false };
      const part = e.over.partition.map((x) => this.ex(x, b, ctx, wp));
      const ord = e.over.order.map((o) => ({ v: this.ex(o.e, b, ctx, wp), dir: o.dir }));
      extra.push(...part, ...ord.map((o) => o.v));
      overSql = ` OVER (${part.length ? 'PARTITION BY ' + part.map((v) => v.sql).join(', ') : ''}${ord.length ? (part.length ? ' ' : '') + 'ORDER BY ' + ord.map((o) => `${o.v.sql} ${o.dir}`).join(', ') : ''}${e.over.frame ? ' ' + e.over.frame : ''})`;
    }
    const s = args.map((a) => a.sql);
    let sql, dtype = args[0] ? args[0].dtype : null;
    switch (name) {
      case 'count': sql = `count(${e.star ? '*' : (e.distinct ? 'DISTINCT ' : '') + s.join(', ')})`; dtype = 'int'; break;
      case 'avg': sql = `avg(${e.distinct ? 'DISTINCT ' : ''}${s.join(', ')})`; dtype = 'float'; break;
      case 'string_agg': sql = `group_concat(${s.join(', ')})`; dtype = 'text'; break;
      case 'bool_and': sql = `min(${s[0]})`; dtype = 'bool'; break;
      case 'bool_or': sql = `max(${s[0]})`; dtype = 'bool'; break;
      case 'array_agg': throw unsupported('array_agg is not supported.');
      case 'length': case 'char_length': case 'character_length': sql = `length(${s[0]})`; dtype = 'int'; break;
      case 'octet_length': sql = `length(CAST(${s[0]} AS BLOB))`; dtype = 'int'; break;
      case 'ceiling': sql = `ceil(${s[0]})`; break;
      case 'substring': sql = `substr(${s.join(', ')})`; dtype = 'text'; break;
      case 'btrim': sql = `trim(${s.join(', ')})`; dtype = 'text'; break;
      case 'concat': sql = '(' + s.map((x) => `COALESCE(CAST(${x} AS TEXT), '')`).join(' || ') + ')'; dtype = 'text'; break;
      case 'greatest': sql = `max(${s.join(', ')})`; break;
      case 'least': sql = `min(${s.join(', ')})`; break;
      case 'date_trunc': sql = `dep_date_trunc(${s.join(', ')})`; dtype = 'timestamp'; break;
      case 'date_part': sql = `dep_date_part(${s.join(', ')})`; dtype = 'int'; break;
      case 'left': sql = `substr(${s[0]}, 1, ${s[1]})`; dtype = 'text'; break;
      case 'right': sql = `substr(${s[0]}, -(${s[1]}))`; dtype = 'text'; break;
      case 'strpos': sql = `instr(${s[0]}, ${s[1]})`; dtype = 'int'; break;
      case 'starts_with': sql = `(substr(${s[0]}, 1, length(${s[1]})) = ${s[1]})`; dtype = 'bool'; break;
      case 'now': sql = "datetime('now','localtime')"; dtype = 'timestamp'; break;
      case 'current_date': sql = "date('now','localtime')"; dtype = 'date'; break;
      case 'reverse': case 'lpad': case 'rpad': throw unsupported(`${name}() is not supported by the simulator.`);
      case 'to_date': sql = `date(${s[0]})`; dtype = 'date'; break;
      case 'to_timestamp': sql = `datetime(${s[0]})`; dtype = 'timestamp'; break;
      case 'sha256': sql = `dep_sha256(${s[0]})`; dtype = 'text'; break;
      case 'md5': throw unsupported('md5() is not supported by the simulator.');
      case 'random': sql = '((random() & 9007199254740991) / 9007199254740992.0)'; dtype = 'float'; break;
      case 'lower': case 'upper': case 'trim': case 'ltrim': case 'rtrim': case 'replace': case 'substr': dtype = 'text'; sql = `${name}(${s.join(', ')})`; break;
      case 'row_number': case 'rank': case 'dense_rank': case 'ntile': dtype = 'int'; sql = `${name}(${s.join(', ')})`; break;
      default: sql = `${name}(${e.distinct ? 'DISTINCT ' : ''}${s.join(', ')})`;
    }
    const opName = isWin ? 'A window function' : isAgg ? `Aggregate ${name}()` : `Function ${name}()`;
    const out = this.generic(opName, args.concat(extra), sql + overSql, { dtype, agg: isAgg || isWin, volatile: name === 'random' || name === 'now' });
    if (name === 'now' || name === 'current_date') out.constant = true;
    if (isWin) out.agg = false, out.win = true;
    return out;
  }

  exReveal(e, b, ctx, pos) {
    if (e.args.length !== 1 || e.star) throw sqlerr('42883', 'REVEAL takes one argument');
    const a = e.args[0];
    if (a.type !== 'col') throw sqlerr('42804', 'REVEAL takes a PROTECTED column');
    const v = this.ex(a, b, ctx, { place: 'project', conj: false });
    if (v.kind !== 'prot') throw sqlerr('42804', 'REVEAL takes a PROTECTED column');
    if (!(ctx.region && ctx.region.discloses.has(v.domain))) throw dep('DEP_REVEAL_OUTSIDE_DISCLOSURE', `REVEAL() of ${v.desc} (match domain ${v.domain}) appears outside a disclosing function.`, `Move the disclosure into a function declared DISCLOSES ${v.domain}.`);
    if (pos.place !== 'project') throw disclosedUse(`REVEAL(${v.desc})`, PLACE[pos.place]);
    if (!v.hasCt) throw unsupported(`REVEAL of ${v.desc} must read the column directly, not through an aggregate or a scalar subquery.`);
    return { kind: 'disc', sql: `dep_decrypt(${ql(v.domain)}, ${v.physCt})`, domain: v.domain, desc: v.desc, dtype: 'text' };
  }

  /* --- FROM items */
  fromItem(it, b, ctx) {
    if (it.kind === 'table') {
      for (let i = ctx.ctes.length - 1; i >= 0; i--) { const c = ctx.ctes[i].get(it.name); if (c) return this.subRel(c.q, it.alias || it.name, ctx, c.cols || it.colAliases); }
      return this.relForTable(it.name, it.alias, ctx);
    }
    if (it.kind === 'sub') return this.subRel(it.q, it.alias, ctx, it.colAliases);
    if (it.kind === 'func') return this.callFunction(it, ctx);
    throw unsupported('This FROM item is not supported.');
  }
  relForTable(name, alias, ctx) {
    const t = this.cat().tables.get(name);
    if (!t) throw sqlerr('42P01', `relation "${name}" does not exist`);
    ctx.uses.tables.add(name);
    const a = alias || name;
    const cols = new Map(), order = [];
    for (const c of t.columns) {
      cols.set(c.name, { name: c.name, kind: c.domain ? 'prot' : 'ord', domain: c.domain, dtype: c.domain ? 'text' : c.kind, phys: `${qi(a)}.${qi(c.name)}`, physCt: c.domain ? `${qi(a)}.${qi(c.name + CT)}` : null, base: { table: t.name, col: c.name }, desc: `${t.name}.${c.name}` });
      order.push(c.name);
    }
    return { alias: a, kind: 'table', sql: `${qi(t.name)} AS ${qi(a)}`, cols, order, tableName: t.name, meta: t };
  }
  subRel(q, alias, ctx, colAliases) {
    const sub = this.compileQuery(q, ctx, {});
    const cols = new Map(), order = [], anchoredOut = [];
    sub.outCols.forEach((oc, i) => {
      const n = colAliases && colAliases[i] ? colAliases[i] : oc.name;
      const src = oc.name;
      cols.set(n, { name: n, kind: oc.kind, domain: oc.domain, dtype: oc.dtype, phys: `${qi(alias)}.${qi(src)}`, physCt: oc.kind === 'prot' && oc.hasCt ? `${qi(alias)}.${qi(src + CT)}` : null, base: null, desc: oc.desc });
      order.push(n);
      if (oc.kind === 'prot' && oc.anchored) anchoredOut.push(n);
    });
    return { alias, kind: 'sub', sql: `(${sub.sql}) AS ${qi(alias)}`, cols, order, anchoredOut };
  }

  callFunction(it, ctx) {
    const cat = this.cat();
    const f = cat.functions.get(it.name);
    if (!f) throw sqlerr('42703', `Error during planning: table function '${it.name}' not found`);
    // The declared-call rule, and the masked read path.
    if (ctx.region) {
      const r = ctx.region.fn;
      const ok = f.discloses.every((d) => r.discloses.includes(d)) && f.links.every((d) => r.links.includes(d));
      if (!ok) throw dep('DEP_DECLARED_CALL', `Function ${r.name} calls ${f.name}(), which is not declared for the same purpose and match domain.`, 'Inline the logic, or declare both functions for the same purpose.');
    } else if (ctx.path === 'MASKED_READ') {
      if (f.discloses.length) throw masked('Disclosing functions are');
      if (!f.maskedOk) throw masked('Linking functions that take a protected input are');
    }
    if (f.discloses.length) { this.note(ctx, `${f.name}() is declared DISCLOSES ${f.discloses.join(', ')}: the columns it declares are returned in plaintext.`); this.event(ctx, 'disclose', `${f.name}()`, f.discloses.join(', '), 0); }
    if (f.links.length) { this.note(ctx, `${f.name}() is declared LINKS ${f.links.join(', ')}: it may join protected values without an anchor.`); this.event(ctx, 'link', `${f.name}()`, f.links.join(', '), 0); }
    ctx.uses.functions.add(f.name);
    if (it.args.length !== f.params.length) throw sqlerr('XX000', `Error during planning: function ${f.name}() takes ${f.params.length} argument(s), got ${it.args.length}`);
    // Arguments are values: they are evaluated before the body runs.
    const empty = this.newBlock(null);
    const actx = this.newCtx({ role: ctx.role, path: ctx.path, region: ctx.region, params: ctx.params, notes: [], events: [] });
    const params = new Map();
    f.params.forEach((p, i) => {
      if (refersToColumns(it.args[i])) throw sqlerr('XX000', `Error during planning: arguments of ${f.name}() must be values`);
      const v = this.ex(it.args[i], empty, actx, { place: 'project', conj: false });
      let value = v.value;
      if (value === undefined || v.castOf || !v.lit) { const res = this.readDb().exec(`SELECT ${castTo(v.sql, p.kind)}`); value = res.length ? res[0].values[0][0] : null; }
      else { const res = this.readDb().exec(`SELECT ${castTo(ql(value), p.kind)}`); value = res.length ? res[0].values[0][0] : null; }
      params.set(p.name, { value, kind: p.kind });
    });
    // The body runs with its owner's privileges; it was analyzed when the function was declared.
    this.checkOwner(cat, f);
    const region = { fn: f, discloses: new Set(f.discloses), links: new Set(f.links), params, paramMatch: false };
    const bctx = this.newCtx({ role: f.owner, path: 'OPERATIONAL', region, scans: ctx.scans, params: [] });
    const body = this.compileQuery(f.ast, bctx, { fnBody: f });
    const alias = it.alias || f.name;
    const cols = new Map(), order = [];
    f.returns.forEach((rt, i) => {
      const n = it.colAliases && it.colAliases[i] ? it.colAliases[i] : rt.name;
      const d = f.disclosed.find(([cn]) => cn === rt.name);
      cols.set(n, { name: n, kind: d ? 'disc' : 'ord', domain: d ? d[1] : null, dtype: rt.kind, phys: `${qi(alias)}.${qi(rt.name)}`, physCt: null, base: null, desc: `Column ${rt.name} of ${f.name}()` });
      order.push(n);
    });
    return { alias, display: it.alias ? null : `${f.name}()`, kind: 'func', sql: `(${body.sql}) AS ${qi(alias)}`, cols, order, fn: f };
  }

  checkOwner(cat, f) {
    for (const [t, c] of f.reads) if (!cat.tables.has(t) || !C.canSelect(cat, f.owner, t, c)) throw denied(`permission denied for function ${f.name}: its owner ${f.owner} can no longer read ${t}.${c}`);
    for (const t of f.scans) if (!cat.tables.has(t) || !C.canSelectAny(cat, f.owner, t)) throw denied(`permission denied for function ${f.name}: its owner ${f.owner} can no longer read ${t}`);
    for (const g of f.calls) { const gm = cat.functions.get(g); if (!gm || !C.canExecute(cat, f.owner, gm)) throw denied(`permission denied for function ${f.name}: its owner ${f.owner} can no longer execute ${g}`); this.checkOwner(cat, gm); }
  }

  /* The privileges a statement needs: every column it names, every table it reads, every function it calls. */
  verifyUses(ctx, role) {
    const cat = this.cat();
    const cols = [...ctx.uses.columns].map((k) => k.split(KEYSEP)).sort(cmpPair);
    for (const [t, c] of cols) {
      if (!C.canSelect(cat, role, t, c)) {
        const prot = cat.tables.has(t) && (C.column(cat.tables.get(t), c) || {}).domain;
        throw denied(`permission denied for column ${c} of table ${t}`, prot ? `In DEP, the privilege on a PROTECTED column is the privilege to match on it. GRANT SELECT (${c}) ON ${t} TO ${role} would allow it.` : `GRANT SELECT (${c}) ON ${t} TO ${role} would allow it.`);
      }
    }
    for (const t of sorted(ctx.uses.tables)) if (!cols.some(([x]) => x === t) && !C.canSelectAny(cat, role, t)) throw denied(`permission denied for table ${t}`);
    for (const f of sorted(ctx.uses.functions)) {
      const m = cat.functions.get(f); if (!m) continue;
      if (!C.canExecute(cat, role, m)) throw denied(`permission denied for function ${f}`, `The owner, ${m.owner}, can GRANT EXECUTE ON FUNCTION ${f} TO ${role}.`);
      this.checkOwner(cat, m);
    }
  }

  /* --- queries */
  compileQuery(q, ctx, o) {
    o = o || {};
    if (q.with && q.with.length) ctx = Object.assign({}, ctx, { ctes: ctx.ctes.concat([new Map(q.with.map((w) => [w.name, w]))]) });
    if (q.type === 'compound') return this.compileCompound(q, ctx, o);
    if (q.type === 'values') return this.compileValues(q, ctx, o);
    return this.compileSelect(q, ctx, o);
  }

  compileValues(q, ctx, o) {
    const b = this.newBlock(o.parent);
    const width = q.rows[0].length;
    const dtypes = [];
    const rows = q.rows.map((r) => {
      if (r.length !== width) throw sqlerr('42601', 'VALUES lists must all be the same length');
      return '(' + r.map((x, i) => { const v = this.ex(x, b, ctx, { place: 'project', conj: false }); dtypes[i] = dtypes[i] || v.dtype; if (v.kind === 'prot') throw protectedOp('VALUES', v.desc); return v.sql; }).join(', ') + ')';
    });
    const outCols = []; const sel = [];
    for (let i = 0; i < width; i++) { const n = o.rename ? o.rename[i] : `column${i + 1}`; outCols.push({ name: n, kind: 'ord', dtype: dtypes[i] }); sel.push(`${qi('column' + (i + 1))} AS ${qi(n)}`); }
    let sql = `SELECT ${sel.join(', ')} FROM (VALUES ${rows.join(', ')})`;
    sql = this.applyTail(q, sql, outCols, ctx);
    return { sql, outCols, block: b };
  }

  compileCompound(q, ctx, o) {
    const L = this.compileQuery(q.l, ctx, { parent: o.parent }), R = this.compileQuery(q.r, ctx, { parent: o.parent });
    for (const side of [L, R]) if (side.outCols.some((c) => c.kind === 'prot')) throw unsupported(`${q.op} over PROTECTED columns is not supported.`);
    if (L.outCols.length !== R.outCols.length) throw sqlerr('42601', `each ${q.op} query must have the same number of columns`);
    const outCols = L.outCols.map((c, i) => { const r = R.outCols[i]; const k = c.kind === 'disc' ? c : r.kind === 'disc' ? r : c; return { name: o.rename ? o.rename[i] : c.name, kind: k.kind, domain: k.domain, desc: k.desc, dtype: c.dtype || r.dtype }; });
    const names = outCols.map((c, i) => `${qi(L.outCols[i].name)} AS ${qi(c.name)}`).join(', ');
    let sql = `SELECT ${names} FROM (SELECT * FROM (${L.sql}) ${q.op}${q.all ? ' ALL' : ''} SELECT * FROM (${R.sql}))`;
    sql = this.applyTail(q, sql, outCols, ctx);
    return { sql, outCols, block: this.newBlock(null) };
  }

  /* ORDER BY, LIMIT and OFFSET over the outputs of a compound query or VALUES. */
  applyTail(q, sql, outCols, ctx) {
    if (!(q.orderBy && q.orderBy.length) && !q.limit && !q.offset) return sql;
    const b = this.newBlock(null);
    const rel = { alias: '__dep_t', kind: 'sub', cols: new Map(), order: [] };
    for (const c of outCols) rel.cols.set(c.name, { name: c.name, kind: c.kind, domain: c.domain, desc: c.desc, dtype: c.dtype, phys: `${qi('__dep_t')}.${qi(c.name)}`, physCt: null, base: null });
    b.rels.push(rel);
    const order = (q.orderBy || []).map((ob) => {
      let e = ob.e;
      if (e.type === 'lit' && e.ltype === 'num') { const oc = outCols[e.v - 1]; if (!oc) throw sqlerr('42P10', `ORDER BY position ${e.v} is not in select list`); e = { type: 'col', table: null, name: oc.name }; }
      const v = this.ex(e, b, ctx, { place: 'sort', conj: false });
      if (v.kind === 'prot') throw protectedOp('ORDER BY', v.desc);
      return `${v.sql} ${ob.dir}${ob.nulls ? ' NULLS ' + ob.nulls : ''}`;
    });
    let out = `SELECT * FROM (${sql}) AS ${qi('__dep_t')}`;
    if (order.length) out += ' ORDER BY ' + order.join(', ');
    out += this.limitSql(q, b, ctx);
    return out;
  }
  limitSql(q, b, ctx) {
    let s = '';
    if (q.limit) { const v = this.ex(q.limit, b, ctx, { place: 'project', conj: false }); s += ` LIMIT ${v.sql}`; }
    if (q.offset) { const v = this.ex(q.offset, b, ctx, { place: 'project', conj: false }); s += `${q.limit ? '' : ' LIMIT -1'} OFFSET ${v.sql}`; }
    return s;
  }

  displayName(e, b) {
    switch (e.type) {
      case 'col': return e.name;
      case 'func': return `${e.name.toLowerCase()}(${e.star ? '*' : (e.distinct ? 'DISTINCT ' : '') + e.args.map((a) => this.displayArg(a, b)).join(', ')})`;
      case 'cast': return this.displayName(e.e, b);
      case 'lit': case 'arith': case 'bin': case 'neg': return this.displayArg(e, b);
      case 'curdate': return 'current_date()';
      case 'curts': return 'now()';
      default: return '?column?';
    }
  }
  displayArg(e, b) {
    switch (e.type) {
      case 'col': { try { const r = this.resolve(b, { region: null }, e.table, e.name); if (r.rel) return `${r.rel.display || r.rel.alias}.${e.name}`; } catch (x) { /* unresolved */ } return e.table ? `${e.table}.${e.name}` : e.name; }
      case 'lit': return e.ltype === 'str' ? `Utf8("${e.v}")` : e.ltype === 'num' ? `Int64(${e.v})` : e.ltype === 'float' ? `Float64(${e.v})` : e.ltype === 'bool' ? `Boolean(${e.v})` : 'NULL';
      case 'func': return this.displayName(e, b);
      case 'cast': return `CAST(${this.displayArg(e.e, b)} AS ${sqlTypeName(e.to)})`;
      case 'arith': case 'bin': return `${this.displayArg(e.l, b)} ${e.op === '<>' ? '!=' : e.op} ${this.displayArg(e.r, b)}`;
      case 'neg': return `(- ${this.displayArg(e.e, b)})`;
      case 'curdate': return 'current_date()';
      default: return '?column?';
    }
  }

  compileSelect(q, ctx, o) {
    const b = this.newBlock(o.parent);
    // FROM, with join conditions
    const froms = [];
    for (const it of q.from) {
      const rel = this.fromItem(it, b, ctx);
      if (b.rels.some((r) => r.alias === rel.alias)) throw sqlerr('42712', `table name "${rel.alias}" specified more than once`);
      let on = it.on;
      if (it.using) {
        const conds = it.using.map((c) => {
          const left = b.rels.find((r) => r.cols.has(c));
          if (!left) throw sqlerr('42703', `column "${c}" specified in USING clause does not exist in left table`);
          if (!rel.cols.has(c)) throw sqlerr('42703', `column "${c}" specified in USING clause does not exist in right table`);
          return { type: 'bin', op: '=', l: { type: 'col', table: left.alias, name: c }, r: { type: 'col', table: rel.alias, name: c } };
        });
        on = conds.reduce((a, c) => (a ? { type: 'bin', op: 'AND', l: a, r: c } : c), null);
        it.using.forEach((c) => b.using.add(c));
      }
      b.rels.push(rel);
      for (const n of rel.anchoredOut || []) b.anchors.add(rel.alias + '.' + n);
      let onSql = null;
      if (on) {
        const v = this.ex(on, b, ctx, { place: 'join', conj: true });
        onSql = v.sql;
        if (it.join === 'INNER') this.addLookups(b, v);
      }
      froms.push({ it, rel, onSql });
    }
    // WHERE
    let whereSql = null;
    if (q.where) { const v = this.ex(q.where, b, ctx, { place: 'filter', conj: true }); whereSql = v.sql; this.addLookups(b, v); }
    // the select list, with stars expanded
    const items = [];
    for (const c of q.cols) {
      if (c.e.type === 'star') {
        const rels = c.e.table ? b.rels.filter((r) => r.alias === c.e.table) : b.rels;
        if (c.e.table && !rels.length) throw sqlerr('42P01', `missing FROM-clause entry for table "${c.e.table}"`);
        if (!rels.length) throw sqlerr('42601', 'SELECT * with no tables specified is not valid');
        const seen = new Set();
        for (const r of rels) for (const n of r.order) {
          if (!c.e.table && b.using.has(n) && seen.has(n)) continue;
          seen.add(n);
          items.push({ e: { type: 'col', table: r.alias, name: n }, name: n });
        }
      } else items.push({ e: c.e, name: c.alias || this.displayName(c.e, b), alias: c.alias });
    }
    // GROUP BY (ordinals and output names refer to the select list)
    const grouped = new Set();
    const groupSql = q.groupBy.map((g) => {
      let ge = g;
      if (g.type === 'lit' && g.ltype === 'num') { const it = items[g.v - 1]; if (!it) throw sqlerr('42P10', `GROUP BY position ${g.v} is not in select list`); ge = it.e; }
      else if (g.type === 'col' && !g.table && !b.rels.some((r) => r.cols.has(g.name))) { const it = items.find((x) => x.alias === g.name); if (it) ge = it.e; }
      const v = this.ex(ge, b, ctx, { place: 'group', conj: false });
      if (v.kind === 'prot') {
        if (!this.links(ctx, v.domain)) throw dep('DEP_UNANCHORED_MATCH', `GROUP BY on PROTECTED column ${v.desc} links every row that shares a value.`, `Group by ordinary columns, or perform the grouping inside a function declared LINKS ${v.domain}.`);
        if (v.vkey) grouped.add(v.vkey);
      }
      return v.sql;
    });
    const aggregated = groupSql.length > 0;
    // the values of the select list
    const vals = items.map((it) => this.ex(it.e, b, ctx, { place: 'project', conj: false }));
    // HAVING
    let havingSql = null;
    if (q.having) { const v = this.ex(q.having, b, ctx, { place: 'filter', conj: true }); havingSql = v.sql; }
    const isAgg = aggregated || vals.some((v) => v.agg) || !!q.having;
    // every join between protected columns must be anchored
    this.closeBlock(b, ctx);
    // outputs
    const top = !!o.top, fnBody = o.fnBody || null;
    const outCols = [], sel = [];
    const names = o.rename || (fnBody ? fnBody.returns.map((r) => r.name) : null);
    if (names && names.length !== items.length) {
      if (fnBody) throw sqlerr('42P13', `the body returns ${items.length} column(s), but RETURNS TABLE declares ${names.length}`);
      throw sqlerr('42601', `INSERT has ${items.length} expressions but ${names.length} target columns`);
    }
    items.forEach((it, i) => {
      const v = vals[i];
      const name = names ? names[i] : it.name;
      const rk = fnBody ? fnBody.returns[i].kind : null;
      if (v.kind === 'prot') {
        const hasCt = v.hasCt && !isAgg && !grouped.has(v.vkey);
        const anchored = !!(v.vkey && this.isAnchored(b, v.vkey)) || !!v.anchored;
        if (top || fnBody) { sel.push(`NULL AS ${qi(name)}`); outCols.push({ name, kind: 'prot', domain: v.domain, desc: v.desc, dtype: rk || 'text', nulled: true }); }
        else {
          sel.push(`${v.sql} AS ${qi(name)}`);
          if (hasCt) sel.push(`${v.physCt} AS ${qi(name + CT)}`);
          outCols.push({ name, kind: 'prot', domain: v.domain, desc: v.desc, hasCt, anchored, dtype: 'text' });
        }
        return;
      }
      const sql = rk ? castTo(v.sql, rk) : v.sql;
      sel.push(`${sql} AS ${qi(name)}`);
      outCols.push({ name, kind: v.kind, domain: v.domain, desc: v.desc, dtype: rk || v.dtype });
    });
    if (q.distinct) for (const oc of outCols) if (oc.kind === 'prot' && !this.links(ctx, oc.domain)) throw dep('DEP_UNANCHORED_MATCH', `DISTINCT on PROTECTED column ${oc.desc} links every row that shares a value.`, `Perform it inside a function declared LINKS ${oc.domain}.`);
    // ORDER BY: output names and ordinals first, then expressions over the input
    const orderSql = q.orderBy.map((ob) => {
      const g = ob.e; const suffix = `${ob.dir}${ob.nulls ? ' NULLS ' + ob.nulls : ''}`;
      let oc = null;
      if (g.type === 'lit' && g.ltype === 'num') { oc = outCols[g.v - 1]; if (!oc) throw sqlerr('42P10', `ORDER BY position ${g.v} is not in select list`); }
      else if (g.type === 'col' && !g.table) { const k = items.findIndex((x) => x.alias === g.name); if (k >= 0) oc = outCols[k]; }
      if (oc) { if (oc.kind === 'prot') throw protectedOp('ORDER BY', oc.desc); return `${qi(oc.name)} ${suffix}`; }
      const v = this.ex(g, b, ctx, { place: 'sort', conj: false });
      if (v.kind === 'prot') throw protectedOp('ORDER BY', v.desc);
      return `${v.sql} ${suffix}`;
    });
    const limit = this.limitSql(q, b, ctx);
    // how each stored table is read (for EXPLAIN)
    this.recordScans(b, ctx);
    let from = '';
    froms.forEach((f, i) => {
      if (i === 0) from = ` FROM ${f.rel.sql}`;
      else if (f.it.join === 'CROSS') from += ` CROSS JOIN ${f.rel.sql}`;
      else from += ` ${f.it.join === 'INNER' ? 'JOIN' : f.it.join + ' JOIN'} ${f.rel.sql} ON ${f.onSql}`;
    });
    const sql = `SELECT ${q.distinct ? 'DISTINCT ' : ''}${sel.join(', ') || 'NULL'}${from}${whereSql ? ' WHERE ' + whereSql : ''}${groupSql.length ? ' GROUP BY ' + groupSql.join(', ') : ''}${havingSql ? ' HAVING ' + havingSql : ''}${orderSql.length ? ' ORDER BY ' + orderSql.join(', ') : ''}${limit}`;
    return { sql, outCols, block: b };
  }

  outputNotes(outCols) {
    const notes = [];
    for (const oc of outCols) {
      if (oc.kind === 'prot') notes.push(`Output column ${oc.name} is PROTECTED column ${oc.desc} (MATCH DOMAIN ${oc.domain}) and is returned as NULL.`);
      else if (oc.kind === 'disc') notes.push(`Output column ${oc.name} is disclosed: ${oc.desc} (DISCLOSES ${oc.domain}).`);
    }
    return notes;
  }

  addLookups(b, v) {
    for (const l of v.lookups || (v.lookup ? [v.lookup] : [])) if (l) b.lookups.push(l);
    for (const q of v.equivs || (v.equiv ? [v.equiv] : [])) if (q) b.equivs.push(q);
  }
  recordScans(b, ctx) {
    // An equality between two columns carries a lookup on one to the other, as the optimizer does.
    const lk = b.lookups.slice();
    for (let changed = true, guard = 0; changed && guard < 20; guard++) {
      changed = false;
      for (const [x, y] of b.equivs) for (const [p, q] of [[x, y], [y, x]]) {
        const src = lk.filter((l) => l.alias === p.alias && l.col === p.col); if (!src.length) continue;
        const k = Math.min(...src.map((s) => s.keys));
        if (!lk.some((l) => l.alias === q.alias && l.col === q.col && l.keys <= k)) { lk.push(Object.assign({}, q, { keys: k })); changed = true; }
      }
    }
    for (const r of b.rels) {
      if (r.kind !== 'table') continue;
      let best = null;
      for (const l of lk) {
        if (l.alias !== r.alias) continue;
        const ix = C.singleIndex(r.meta, l.col); if (!ix) continue;
        if (!best || l.keys < best.keys) best = { index: ix.name, keys: l.keys };
      }
      ctx.scans.push(best ? `${r.tableName}: index ${best.index}, ${best.keys} key(s)` : `${r.tableName}: every row`);
    }
  }

  /* ================================================================ statements */

  publish(ctx, outCols) { this.addNotes(ctx.notes.concat(outCols ? this.outputNotes(outCols) : [])); }

  runRows(sql, outCols, ctx) {
    ctx.compiled.push(sql);
    const res = this.readDb().exec(sql);
    let rows = res.length ? res[0].values : [];
    const boolCols = outCols.map((c, i) => (c.dtype === 'bool' ? i : -1)).filter((i) => i >= 0);
    if (boolCols.length) rows = rows.map((r) => { const x = r.slice(); for (const i of boolCols) if (x[i] !== null) x[i] = !!x[i]; return x; });
    return rows;
  }

  execQuery(ast, ctx) {
    const c = this.compileQuery(ast, ctx, { top: true });
    this.verifyUses(ctx, this.role());
    this.publish(ctx, c.outCols);
    this.admitEvents(ctx.events);
    const rows = this.runRows(c.sql, c.outCols, ctx);
    return { kind: 'rows', columns: c.outCols.map((oc) => ({ name: oc.name, tag: oc.kind === 'prot' ? 'protected' : oc.kind === 'disc' ? 'disclosed' : null, domain: oc.domain || null })), rows, tag: `SELECT ${rows.length}` };
  }

  execExplain(ast, ctx) {
    const inner = ast.inner;
    if (!inner || !['select', 'compound', 'values'].includes(inner.type)) throw unsupported('EXPLAIN supports queries.');
    const c = this.compileQuery(inner, ctx, { top: true });
    this.verifyUses(ctx, this.role());
    this.publish(ctx, c.outCols);
    this.admitEvents(ctx.events);
    const lines = ['SQLite: ' + c.sql];
    if (ctx.scans.length) { lines.push('Read:'); for (const s of ctx.scans) lines.push('  ' + s); }
    return { kind: 'rows', columns: [{ name: 'dep_plan', tag: null }], rows: lines.map((l) => [l]), tag: `EXPLAIN` };
  }

  /* --- constraints, checked after a statement's writes and before they are kept */
  showValue(v) { return v === null || v === undefined ? 'NULL' : String(v); }
  checkNotNull(t) {
    for (const c of t.columns) if (c.notNull) {
      const r = this.db.exec(`SELECT 1 FROM ${qi(t.name)} WHERE ${qi(c.name)} IS NULL LIMIT 1`);
      if (r.length) throw sqlerr('23502', `null value in column "${c.name}" of relation "${t.name}" violates not-null constraint`);
    }
  }
  describeKey(t, ix, vals) {
    const cols = ix.columns.join(', ');
    if (ix.columns.some((c) => (C.column(t, c) || {}).domain)) return `Key (${cols})`;
    return `Key (${cols})=(${vals.map((v) => this.showValue(v)).join(', ')})`;
  }
  checkUnique(t, only) {
    for (const ix of t.indexes) {
      if (!ix.unique || !ix.enforced || (only && only !== ix.name)) continue;
      const cols = ix.columns.map(qi).join(', ');
      const r = this.db.exec(`SELECT ${cols} FROM ${qi(t.name)} WHERE ${ix.columns.map((c) => `${qi(c)} IS NOT NULL`).join(' AND ')} GROUP BY ${cols} HAVING count(*) > 1 LIMIT 1`);
      if (r.length) throw sqlerr('23505', `duplicate key value violates unique constraint "${ix.name}": ${this.describeKey(t, ix, r[0].values[0])} already exists.`);
    }
  }
  checkRefsOut(t) {
    const cat = this.cat();
    for (const c of t.columns) {
      if (!c.references) continue;
      const p = cat.tables.get(c.references.table); if (!p) throw sqlerr('42P01', `relation "${c.references.table}" does not exist`);
      const pc = c.references.col;
      const r = this.db.exec(`SELECT ${qi(c.name)} FROM ${qi(t.name)} WHERE ${qi(c.name)} IS NOT NULL AND ${qi(c.name)} NOT IN (SELECT ${qi(pc)} FROM ${qi(p.name)} WHERE ${qi(pc)} IS NOT NULL) LIMIT 1`);
      if (r.length) {
        const shown = c.domain ? `Key (${c.name})` : `Key (${c.name})=(${this.showValue(r[0].values[0][0])})`;
        throw sqlerr('23503', `insert or update on table "${t.name}" violates foreign key constraint "${t.name}_${c.name}_fkey": ${shown} is not present in table "${p.name}".`);
      }
    }
  }
  checkRefsIn(t) {
    const cat = this.cat();
    for (const [child, ccol, pcol] of C.referencesTo(cat, t.name)) {
      const r = this.db.exec(`SELECT ${qi(ccol)} FROM ${qi(child)} WHERE ${qi(ccol)} IS NOT NULL AND ${qi(ccol)} NOT IN (SELECT ${qi(pcol)} FROM ${qi(t.name)} WHERE ${qi(pcol)} IS NOT NULL) LIMIT 1`);
      if (r.length) {
        const prot = (C.column(t, pcol) || {}).domain;
        const shown = prot ? `Key (${pcol})` : `Key (${pcol})=(${this.showValue(r[0].values[0][0])})`;
        throw sqlerr('23503', `update or delete on table "${t.name}" violates foreign key constraint "${child}_${ccol}_fkey" on table "${child}": ${shown} is still referenced from table "${child}".`);
      }
    }
  }

  /* --- INSERT */
  execInsert(a, ctx) {
    if (a.bad) throw unsupported(a.bad);
    const name = a.table;
    const meta0 = this.cat().tables.get(name);
    if (!meta0) throw sqlerr('42P01', `relation "${name}" does not exist`);
    const targets = a.cols || meta0.columns.map((c) => c.name);
    for (const t of targets) if (!C.column(meta0, t)) throw sqlerr('42703', `column "${t}" of relation "${name}" does not exist`);
    const t = this.writeTable(name);
    const src = this.compileQuery(a.q, ctx, {});
    if (!C.canInsert(this.cat(), this.role(), name)) throw denied(`permission denied for table ${name}`);
    this.verifyUses(ctx, this.role());
    const stored = t.columns.filter((c) => c.domain && targets.includes(c.name)).map((c) => `PROTECTED column ${name}.${c.name} (MATCH DOMAIN ${c.domain}) is stored as a blind index and ciphertext.`);
    this.addNotes(ctx.notes.concat(stored));
    if (src.outCols.length !== targets.length) throw sqlerr('42601', `INSERT has ${src.outCols.length} expressions but ${targets.length} target columns`);
    const cols = [], exprs = [];
    for (const c of t.columns) {
      const j = targets.indexOf(c.name); if (j < 0) continue;
      const oc = src.outCols[j]; const s = `${qi('__dep_s')}.${qi(oc.name)}`;
      if (oc.kind === 'disc') throw disclosedUse(oc.desc, 'a stored value');
      if (!c.domain) {
        if (oc.kind === 'prot') throw dep('DEP_PROTECTED_OPERATION', `Storing PROTECTED value ${oc.desc} in the ordinary column ${name}.${c.name} is forbidden.`, 'To release a value, use a disclosing function.');
        cols.push(qi(c.name)); exprs.push(castTo(s, c.kind));
      } else if (oc.kind === 'prot') {
        if (oc.domain !== c.domain) throw dep('DEP_MATCH_DOMAIN_MISMATCH', `${oc.desc} (match domain ${oc.domain}) cannot be stored in ${name}.${c.name} (match domain ${c.domain}).`);
        if (!oc.hasCt) throw unsupported('Copy protected values directly from a table column.');
        cols.push(qi(c.name), qi(c.name + CT)); exprs.push(s, `${qi('__dep_s')}.${qi(oc.name + CT)}`);
      } else {
        cols.push(qi(c.name), qi(c.name + CT));
        exprs.push(`dep_index(${ql(c.domain)}, ${textOf(s)})`, `dep_encrypt(${ql(c.domain)}, ${textOf(s)})`);
      }
    }
    this.admitEvents(ctx.events);
    const sql = `INSERT INTO ${qi(name)} (${cols.join(', ')}) SELECT ${exprs.join(', ') || 'NULL'} FROM (${src.sql}) AS ${qi('__dep_s')}`;
    ctx.compiled.push(sql);
    if (!cols.length) this.db.run(`INSERT INTO ${qi(name)} DEFAULT VALUES`); else this.db.run(sql);
    const n = this.db.getRowsModified();
    this.checkNotNull(t); this.checkUnique(t); this.checkRefsOut(t);
    return { kind: 'command', tag: `INSERT 0 ${n}` };
  }

  /* The target of an UPDATE or DELETE, as a relation whose columns name the stored table. */
  dmlRel(t, alias, ctx) {
    const rel = this.relForTable(t.name, alias, ctx);
    for (const c of rel.cols.values()) { c.phys = `${qi(t.name)}.${qi(c.name)}`; c.physCt = c.physCt ? `${qi(t.name)}.${qi(c.name + CT)}` : null; }
    return rel;
  }

  execUpdate(a, ctx) {
    if (a.bad) throw unsupported(a.bad);
    const name = a.table;
    if (!this.cat().tables.has(name)) throw sqlerr('42P01', `relation "${name}" does not exist`);
    const t = this.writeTable(name);
    for (const s of a.sets) if (!C.column(t, s.col)) throw sqlerr('42703', `column "${s.col}" of relation "${name}" does not exist`);
    const b = this.newBlock(null); b.rels.push(this.dmlRel(t, a.alias, ctx));
    let vals, hit;
    try {
      vals = a.sets.map((s) => this.ex(s.e, b, ctx, { place: 'project', conj: false }));
      hit = a.where ? this.ex(a.where, b, ctx, { place: 'project', conj: false }) : null;
      this.closeBlock(b, ctx);
      this.verifyUses(ctx, this.role());
    } finally { this.addNotes(ctx.notes); }
    this.admitEvents(ctx.events);
    const cat = this.cat(), role = this.role();
    for (const s of a.sets) if (!C.canUpdate(cat, role, name, s.col)) throw denied(`permission denied for column ${s.col} of table ${name}`, `GRANT UPDATE (${s.col}) ON ${name} TO ${role} would allow it.`);
    const sets = [];
    a.sets.forEach((s, i) => {
      const c = C.column(t, s.col), v = vals[i];
      if (v.kind === 'disc') throw disclosedUse(v.desc, 'a stored value');
      if (!c.domain) {
        if (v.kind === 'prot') throw dep('DEP_PROTECTED_OPERATION', `Storing PROTECTED value ${v.desc} in the ordinary column ${name}.${c.name} is forbidden.`);
        sets.push(`${qi(c.name)} = ${castTo(v.sql, c.kind)}`);
      } else if (v.kind === 'prot') {
        if (v.domain !== c.domain) throw dep('DEP_MATCH_DOMAIN_MISMATCH', `${v.desc} (match domain ${v.domain}) cannot be stored in ${name}.${c.name} (match domain ${c.domain}).`);
        if (!v.vkey || !v.hasCt) throw unsupported('Copy protected values directly from a column.');
        sets.push(`${qi(c.name)} = ${v.sql}`, `${qi(c.name + CT)} = ${v.physCt}`);
      } else sets.push(`${qi(c.name)} = dep_index(${ql(c.domain)}, ${textOf(v.sql)})`, `${qi(c.name + CT)} = dep_encrypt(${ql(c.domain)}, ${textOf(v.sql)})`);
    });
    if (hit && (hit.kind === 'prot' || hit.kind === 'disc')) throw protectedOp('A bare column as a condition', hit.desc);
    const sql = `UPDATE ${qi(name)} SET ${sets.join(', ')}${hit ? ` WHERE (${hit.sql}) IS TRUE` : ''}`;
    ctx.compiled.push(sql);
    this.db.run(sql);
    const n = this.db.getRowsModified();
    this.checkNotNull(t); this.checkUnique(t);
    if (t.columns.some((c) => c.references)) this.checkRefsOut(t);
    this.checkRefsIn(t);
    return { kind: 'command', tag: `UPDATE ${n}` };
  }

  execDelete(a, ctx) {
    if (a.bad) throw unsupported(a.bad);
    const name = a.table;
    const t = this.writeTable(name);
    const b = this.newBlock(null); b.rels.push(this.dmlRel(t, a.alias, ctx));
    let hit;
    try {
      hit = a.where ? this.ex(a.where, b, ctx, { place: 'project', conj: false }) : null;
      this.closeBlock(b, ctx);
      this.verifyUses(ctx, this.role());
    } finally { this.addNotes(ctx.notes); }
    this.admitEvents(ctx.events);
    if (hit && (hit.kind === 'prot' || hit.kind === 'disc')) throw protectedOp('A bare column as a condition', hit.desc);
    if (!C.canDelete(this.cat(), this.role(), name)) throw denied(`permission denied for table ${name}`);
    const sql = `DELETE FROM ${qi(name)}${hit ? ` WHERE (${hit.sql}) IS TRUE` : ''}`;
    ctx.compiled.push(sql);
    this.db.run(sql);
    const n = this.db.getRowsModified();
    this.checkRefsIn(t);
    return { kind: 'command', tag: `DELETE ${n}` };
  }

  /* --- DDL */
  execCreateTable(a, ctx) {
    if (a.asQuery) throw unsupported('CREATE TABLE ... AS is not supported. Create the table, then INSERT ... SELECT.');
    const name = a.name;
    const cat = this.cat();
    if (cat.tables.has(name)) { if (a.ifNotExists) return { kind: 'command', tag: 'CREATE TABLE' }; throw sqlerr('42P07', `relation "${name}" already exists`); }
    const uniques = []; let tablePk = [];
    for (const tc of a.tcons) {
      if (tc.k === 'fk') throw unsupported('Declare references on the column (col TYPE REFERENCES t (c)), not as a table constraint.');
      if (tc.k === 'pk') tablePk = tc.cols;
      else if (tc.k === 'unique') uniques.push(tc.cols);
      else throw unsupported(`Table constraint not supported: ${tc.text}`);
    }
    const columns = [];
    for (const cd of a.cols) {
      const kind = typeOrError(cd.typeText);
      let pk = tablePk.includes(cd.name), notNull = false, references = null;
      for (const o of cd.opts) {
        if (o.k === 'pk') pk = true;
        else if (o.k === 'notnull') notNull = true;
        else if (o.k === 'unique') uniques.push([cd.name]);
        else if (o.k === 'ref') {
          for (const act of o.actions) if (!['NO ACTION', 'RESTRICT'].includes(act.a)) throw unsupported(`ON DELETE / ON UPDATE ${act.a} is not supported: references are checked (NO ACTION).`);
          if (!o.col) throw sqlerr('42601', 'REFERENCES needs a column');
          references = { table: o.table, col: o.col };
        } else if (o.k === 'default') throw unsupported('DEFAULT values are not supported.');
        else if (o.k === 'check') throw unsupported('CHECK constraints are not supported.');
      }
      let domain = cd.protected ? cd.domain : null;
      if (references) {
        const rt = references.table, rc = references.col;
        const rmeta = rt === name ? null : cat.tables.get(rt);
        if (rt !== name && !rmeta) throw sqlerr('42P01', `relation "${rt}" does not exist`);
        if (rmeta) {
          if (rmeta.temporary && !a.temporary) throw sqlerr('42P16', `constraints on permanent tables may reference only permanent tables (${rt} is TEMPORARY)`);
          const rcol = C.column(rmeta, rc);
          if (!rcol) throw sqlerr('42703', `column "${rc}" referenced in foreign key constraint does not exist`);
          if (!cd.protected && rcol.domain) throw dep('DEP_REFERENCE_MISMATCH', `Column ${name}.${cd.name} references PROTECTED column ${rt}.${rc} (match domain ${rcol.domain}) but is not PROTECTED.`, `Declare it ${cd.typeText} PROTECTED REFERENCES ${rt} (${rc}), so the reference is kept as an index entry.`);
          if (cd.protected && !rcol.domain) throw dep('DEP_REFERENCE_MISMATCH', `Column ${name}.${cd.name} is PROTECTED but references the ordinary column ${rt}.${rc}.`, 'A reference and the column it references must have the same protection and match domain.');
          if (cd.protected && rcol.domain) {
            if (domain && domain !== rcol.domain) throw dep('DEP_MATCH_DOMAIN_MISMATCH', `Column ${name}.${cd.name} (match domain ${domain}) references ${rt}.${rc} (match domain ${rcol.domain}).`, 'A reference and the column it references must have the same match domain.');
            domain = rcol.domain;
          }
        }
      }
      if (cd.protected) {
        if (kind !== 'text') throw sqlerr('42804', `PROTECTED column ${cd.name} must be TEXT`);
        if (!domain) throw sqlerr('42601', `PROTECTED column ${cd.name} needs a match domain`, 'Write TEXT PROTECTED MATCH DOMAIN name, or PROTECTED REFERENCES a protected column.');
      }
      columns.push({ name: cd.name, type: cd.typeText, kind, domain: domain || null, pk, references, notNull: notNull || pk });
    }
    for (const c of tablePk.concat(...uniques)) if (!columns.some((x) => x.name === c)) throw sqlerr('42703', `column "${c}" named in key does not exist`);
    const meta = { name, tid: this.nextTid++, columns, indexes: [], temporary: !!a.temporary };
    deriveIndexes(meta);
    for (const u of uniques) {
      if (meta.indexes.some((i) => i.unique && i.columns.join('\u0001') === u.join('\u0001'))) continue;
      const iname = `${name}_${u.join('_')}_key`;
      const ex = meta.indexes.find((i) => i.columns.join('\u0001') === u.join('\u0001') && !i.unique);
      if (ex) { ex.unique = true; ex.name = iname; } else meta.indexes.push({ name: iname, columns: u, unique: true, primary: false, auto: true, enforced: true });
    }
    const parents = [];
    for (const c of columns) {
      if (!c.references) continue;
      if (c.references.table === name) ensureIndex(meta, c.references.col);
      else { const p = cat.tables.get(c.references.table); if (p && !C.singleIndex(p, c.references.col)) parents.push(c.references); }
    }
    const sql = `CREATE TABLE ${qi(name)} (${columns.map((c) => (c.domain ? `${qi(c.name)} TEXT, ${qi(c.name + CT)} TEXT` : `${qi(c.name)} ${SQLITE_TYPE[c.kind]}`)).join(', ')})`;
    ctx.compiled.push(sql);
    this.db.run(sql);
    this.catMut((cm) => {
      for (const p of parents) { const pt = cm.tables.get(p.table); ensureIndex(pt, p.col); this.s.txn.written.add(pt.tid); }
      cm.tables.set(name, meta);
    });
    this.s.txn.written.add(meta.tid);
    if (meta.temporary) this.addNotes([`${name} is a TEMPORARY table: it is kept in memory only, and is gone when the server stops.`]);
    return { kind: 'command', tag: 'CREATE TABLE' };
  }

  execCreateIndex(a) {
    if (a.partial) throw unsupported('Partial indexes and INCLUDE are not supported.');
    if (a.exprCol) throw unsupported(`Index on an expression (${a.exprCol}) is not supported: index columns.`);
    const cat = this.cat();
    const meta = cat.tables.get(a.table);
    if (!meta) throw sqlerr('42P01', `relation "${a.table}" does not exist`);
    for (const c of a.cols) if (!C.column(meta, c)) throw sqlerr('42703', `column "${c}" does not exist`);
    const name = a.name || `${a.table}_${a.cols.join('_')}_idx`;
    if (C.indexOwner(cat, name) || cat.tables.has(name)) {
      if (a.ifNotExists) { this.addNotes([`Index ${name} already exists; nothing was done.`]); return { kind: 'command', tag: 'CREATE INDEX' }; }
      throw sqlerr('42P07', `relation "${name}" already exists`);
    }
    this.writeTable(a.table);
    const ix = { name, columns: a.cols.slice(), unique: a.unique, primary: false, auto: false, enforced: true };
    const m = cloneCatalog({ t: meta }).t; m.indexes.push(ix);
    if (a.unique) { try { this.checkUnique(m, name); } catch (e) { if (e.sqlstate === '23505') e.hint = 'The table has rows with equal keys; remove the duplicates first.'; throw e; } }
    const prot = a.cols.filter((c) => (C.column(meta, c) || {}).domain);
    if (prot.length) this.addNotes([`Index ${name} keys PROTECTED column(s) ${prot.join(', ')} by their blind index entries: it answers equality only, and holds no plaintext.`]);
    this.catMut((cm) => { cm.tables.set(a.table, m); });
    return { kind: 'command', tag: 'CREATE INDEX' };
  }

  execReindex(a) {
    const cat = this.cat();
    let table = a.name, only = null;
    if (a.kind === 'INDEX') { const o = C.indexOwner(cat, a.name); if (!o) throw sqlerr('42704', `index "${a.name}" does not exist`); table = o.name; only = a.name; }
    const meta = this.writeTable(table);
    const m = cloneCatalog({ t: meta }).t; const notes = [];
    for (const ix of m.indexes) {
      if (only && ix.name !== only) continue;
      if (!ix.unique) continue;
      const cols = ix.columns.map(qi).join(', ');
      const r = this.db.exec(`SELECT count(*) FROM (SELECT 1 FROM ${qi(table)} WHERE ${ix.columns.map((c) => `${qi(c)} IS NOT NULL`).join(' AND ')} GROUP BY ${cols} HAVING count(*) > 1)`);
      const dups = r.length ? r[0].values[0][0] : 0;
      const was = ix.enforced; ix.enforced = dups === 0;
      if (dups > 0) notes.push(`${ix.name}: ${dups} duplicate key(s); uniqueness is still not enforced.`);
      else if (!was) notes.push(`${ix.name}: no duplicate keys; uniqueness is enforced again.`);
    }
    this.addNotes(notes);
    this.catMut((cm) => { cm.tables.set(table, m); });
    return { kind: 'command', tag: 'REINDEX' };
  }

  execCreateEndpoint(a) {
    if (!['OPERATIONAL', 'MASKED_READ'].includes(a.mode)) throw sqlerr('22023', `invalid endpoint mode "${a.mode}"`, 'The modes are OPERATIONAL and MASKED_READ.');
    this.catMut((cm) => {
      if (cm.endpoints.has(a.name)) throw sqlerr('42710', `endpoint "${a.name}" already exists`);
      cm.endpoints.set(a.name, a.mode);
    });
    return { kind: 'command', tag: 'CREATE ENDPOINT' };
  }

  execCreateFunction(a, ctx) {
    if (a.language && a.language !== 'sql') throw dep('DEP_DECLARED_LANGUAGE', `Function ${a.name} is written in LANGUAGE ${a.language}.`, 'Declared functions must be LANGUAGE sql, so the engine can analyze their bodies.');
    const owner = this.role();
    const cat = this.cat();
    const old = cat.functions.get(a.name);
    if (old) {
      if (!a.orReplace) throw sqlerr('42723', `function "${a.name}" already exists`, 'Use CREATE OR REPLACE FUNCTION.');
      if (old.owner !== owner && !C.isSuper(cat, owner)) throw denied(`must be owner of function ${a.name}`);
    }
    for (const [list, kind, word] of [[a.discloses, 'disclose', 'DISCLOSE'], [a.links, 'link', 'LINK']]) {
      for (const d of list) if (!C.hasDomainPriv(cat, owner, d, kind)) throw dep('DEP_DECLARATION_PRIVILEGE', `Role ${owner} may not declare functions that ${word}S match domain ${d}.`, `A superuser can allow it: GRANT ${word} ON MATCH DOMAIN ${d} TO ${owner}.`);
    }
    const known = C.domains(cat);
    for (const d of a.discloses.concat(a.links)) if (!known.has(d)) throw sqlerr('42704', `match domain "${d}" does not exist`);
    const params = a.params.map((p) => ({ name: p.name, type: p.typeText, kind: typeOrError(p.typeText) }));
    const returns = a.returns.map((r) => ({ name: r.name, type: r.typeText, kind: typeOrError(r.typeText) }));
    let ast;
    {
      const btoks = tokenize(a.body).filter((x) => !(x.t === 'op' && x.v === ';'));
      for (let i = 0; i < btoks.length; i++) {
        const w = btoks[i]; if (w.t !== 'id') continue; const v = w.v.toLowerCase();
        if (v.endsWith(CT)) throw sqlerr('42703', `column "${w.v}" does not exist`);
        if (v.startsWith('dep_') && btoks[i + 1] && btoks[i + 1].t === 'op' && btoks[i + 1].v === '(') throw sqlerr('42883', `function ${w.v}() does not exist`);
      }
      const bp = new Parser(btoks); ast = bp.statement();
      if (!ast || !['select', 'compound', 'values'].includes(ast.type) || !bp.eof()) throw unsupported('The body of a declared function must be a single SELECT.');
    }
    const f = { name: a.name, params, returns, discloses: a.discloses.slice(), links: a.links.slice(), body: a.body.trim(), ast, owner, comment: old ? old.comment : null, maskedOk: true, disclosed: [], reads: [], scans: [], calls: [] };
    const pm = new Map(); params.forEach((p) => pm.set(p.name, { value: null, kind: p.kind }));
    const region = { fn: f, discloses: new Set(f.discloses), links: new Set(f.links), params: pm, paramMatch: false };
    const bctx = this.newCtx({ role: owner, path: 'OPERATIONAL', region });
    const body = this.compileQuery(ast, Object.assign(bctx, {}), { fnBody: f });
    this.verifyUses(bctx, owner);
    body.outCols.forEach((oc, i) => { if (oc.kind === 'disc') f.disclosed.push([returns[i].name, oc.domain]); });
    f.maskedOk = f.discloses.length === 0 && !region.paramMatch;
    const cols = [...bctx.uses.columns].map((k) => k.split(KEYSEP)).sort(cmpPair);
    f.reads = cols;
    f.scans = sorted(bctx.uses.tables).filter((t) => !cols.some(([x]) => x === t));
    f.calls = sorted(bctx.uses.functions);
    ctx.compiled.push('-- body: ' + body.sql);
    this.catMut((cm) => {
      cm.functionOrder = cm.functionOrder.filter((n) => n !== f.name); cm.functionOrder.push(f.name);
      cm.functions.set(f.name, f);
    });
    return { kind: 'command', tag: 'CREATE FUNCTION' };
  }

  execComment(a) {
    const role = this.role();
    this.catMut((cm) => {
      const f = cm.functions.get(a.fn);
      if (!f) throw sqlerr('42883', `function ${a.fn} does not exist`);
      if (f.owner !== role && !C.isSuper(cm, role)) throw denied(`must be owner of function ${a.fn}`);
      f.comment = a.text;
    });
    return { kind: 'command', tag: 'COMMENT' };
  }

  execDrop(a) {
    const role = this.role();
    const kind = a.kind === 'USER' ? 'ROLE' : a.kind;
    if (a.kind !== 'FUNCTION' && !C.isSuper(this.cat(), role)) throw denied(`permission denied to drop ${a.kind.toLowerCase()}`, 'Only a superuser can do this.');
    const name = a.name;
    this.catMut((cm) => {
      switch (kind) {
        case 'ROLE':
          if (name === ADMIN || name === role) throw sqlerr('55006', `role "${name}" cannot be dropped`);
          if (!cm.roles.delete(name)) { if (a.ifExists) return; throw sqlerr('42704', `role "${name}" does not exist`); }
          for (const k of [...cm.tablePrivs.keys()]) if (k.split(KEYSEP)[0] === name) cm.tablePrivs.delete(k);
          for (const k of [...cm.domainPrivs.keys()]) if (k.split(KEYSEP)[0] === name) cm.domainPrivs.delete(k);
          for (const s of cm.endpointGrants.values()) s.delete(name);
          for (const s of cm.fnExecute.values()) s.delete(name);
          return;
        case 'TABLE': {
          if (!cm.tables.has(name)) { if (a.ifExists) return; throw sqlerr('42P01', `table "${name}" does not exist`); }
          const dep2 = C.referencesTo(cm, name).find(([c]) => c !== name);
          if (dep2) throw sqlerr('2BP01', `cannot drop table ${name} because other objects depend on it: ${dep2[0]}.${dep2[1]} references it`, `Drop ${dep2[0]} first.`);
          this.s.txn.written.add(cm.tables.get(name).tid);
          cm.tables.delete(name);
          for (const k of [...cm.tablePrivs.keys()]) if (k.split(KEYSEP)[1] === name) cm.tablePrivs.delete(k);
          this.db.run(`DROP TABLE ${qi(name)}`);
          return;
        }
        case 'INDEX': {
          const owner = C.indexOwner(cm, name);
          if (!owner) { if (a.ifExists) return; throw sqlerr('42704', `index "${name}" does not exist`); }
          const ix = owner.indexes.find((i) => i.name === name);
          if (ix.auto) throw sqlerr('2BP01', `cannot drop index ${name}: the engine keeps it for a key, a reference or a PROTECTED column of ${owner.name}`);
          owner.indexes = owner.indexes.filter((i) => i.name !== name);
          return;
        }
        case 'FUNCTION': {
          const f = cm.functions.get(name);
          if (f && f.owner !== role && !C.isSuper(cm, role)) throw denied(`must be owner of function ${name}`);
          cm.fnExecute.delete(name);
          cm.functionOrder = cm.functionOrder.filter((n) => n !== name);
          if (!cm.functions.delete(name)) { if (a.ifExists) return; throw sqlerr('42883', `function ${name} does not exist`); }
          return;
        }
        case 'ENDPOINT':
          if (!cm.endpoints.delete(name) && !a.ifExists) throw sqlerr('42704', `endpoint "${name}" does not exist`);
          return;
        default: throw unsupported(`DROP ${a.kind} is not supported.`);
      }
    });
    return { kind: 'command', tag: `DROP ${a.kind}` };
  }

  /* --- roles and privileges */
  execRole(a) {
    const role = this.role(), o = a.o, name = a.name;
    return this.catMut((cm) => {
      const su = C.isSuper(cm, role);
      const ownPassword = a.verb === 'ALTER' && name === role && o.login === undefined && o.superuser === undefined && o.matchLimit === undefined && o.password !== undefined;
      if (!su && !ownPassword) throw denied(`permission denied to ${a.verb.toLowerCase()} role`, 'Only a superuser can do this.');
      if (a.verb === 'CREATE') {
        if (cm.roles.has(name) || name === PUBLIC) throw sqlerr('42710', `role "${name}" already exists`);
        cm.roles.set(name, { name, login: !!o.login, superuser: !!o.superuser, password: o.password === undefined ? null : o.password, matchLimit: o.matchLimit || { kind: 'default' } });
        return { kind: 'command', tag: 'CREATE ROLE' };
      }
      const r = cm.roles.get(name); if (!r) throw sqlerr('42704', `role "${name}" does not exist`);
      if (o.login !== undefined) r.login = o.login;
      if (o.superuser !== undefined) { if (name === ADMIN && !o.superuser) throw sqlerr('55006', 'the admin role must stay a superuser'); r.superuser = o.superuser; }
      if (o.password !== undefined) r.password = o.password;
      if (o.matchLimit) r.matchLimit = o.matchLimit;
      return { kind: 'command', tag: 'ALTER ROLE' };
    });
  }

  execGrant(g) {
    const role = this.role();
    const tag = g.grant ? 'GRANT' : 'REVOKE';
    this.catMut((cm) => {
      const su = C.isSuper(cm, role);
      for (const r of g.roles) if (r !== PUBLIC && !cm.roles.has(r)) throw sqlerr('42704', `role "${r}" does not exist`);
      const wrong = (p, what) => sqlerr('0LP01', `invalid privilege type ${p.p} for ${what}`);
      const o = g.object;
      if (o.kind === 'table') {
        for (const t of o.names) {
          if (!su) throw denied(`permission denied for table ${t}`, 'Only a superuser grants table privileges.');
          const meta = cm.tables.get(t); if (!meta) throw sqlerr('42P01', `relation "${t}" does not exist`);
          for (const p of g.privs) if (p.cols) for (const c of p.cols) if (!C.column(meta, c)) throw sqlerr('42703', `column "${c}" of relation "${t}" does not exist`);
          for (const r of g.roles) {
            const k = r + KEYSEP + t;
            if (!cm.tablePrivs.has(k)) cm.tablePrivs.set(k, { selectAll: false, selectCols: new Set(), insert: false, updateAll: false, updateCols: new Set(), delete: false });
            const e = cm.tablePrivs.get(k);
            for (const p of g.privs) {
              const v = g.grant;
              if (p.p === 'SELECT' && !p.cols) { e.selectAll = v; if (!v) e.selectCols.clear(); }
              else if (p.p === 'SELECT') p.cols.forEach((c) => (v ? e.selectCols.add(c) : e.selectCols.delete(c)));
              else if (p.p === 'UPDATE' && !p.cols) { e.updateAll = v; if (!v) e.updateCols.clear(); }
              else if (p.p === 'UPDATE') p.cols.forEach((c) => (v ? e.updateCols.add(c) : e.updateCols.delete(c)));
              else if (p.p === 'INSERT') e.insert = v;
              else if (p.p === 'DELETE') e.delete = v;
              else if (p.p === 'ALL') Object.assign(e, { selectAll: v, insert: v, updateAll: v, delete: v, selectCols: new Set(), updateCols: new Set() });
              else throw wrong(p, 'table');
            }
          }
        }
      } else if (o.kind === 'function') {
        const f = cm.functions.get(o.name); if (!f) throw sqlerr('42883', `function ${o.name} does not exist`);
        if (f.owner !== role && !su) throw denied(`permission denied for function ${o.name}`, 'Only the owner of a function, or a superuser, grants EXECUTE on it.');
        for (const p of g.privs) if (!['EXECUTE', 'ALL'].includes(p.p)) throw wrong(p, 'function');
        if (g.grant && (f.discloses.length || f.links.length) && g.roles.includes(PUBLIC)) throw dep('DEP_PUBLIC_EXECUTE', `Function ${o.name} is declared DISCLOSES or LINKS, and cannot be executable by PUBLIC.`, 'Grant EXECUTE to the specific roles that need it.');
        if (!cm.fnExecute.has(o.name)) cm.fnExecute.set(o.name, new Set());
        const s = cm.fnExecute.get(o.name); for (const r of g.roles) (g.grant ? s.add(r) : s.delete(r));
      } else if (o.kind === 'domain') {
        if (!su) throw denied(`permission denied for match domain ${o.name}`, 'Only a superuser grants DISCLOSE and LINK.');
        if (!C.domains(cm).has(o.name)) throw sqlerr('42704', `match domain "${o.name}" does not exist`);
        if (g.grant && g.roles.includes(PUBLIC)) throw dep('DEP_DECLARATION_PRIVILEGE', 'DISCLOSE and LINK cannot be granted to PUBLIC.', 'Grant them to the roles that declare functions.');
        for (const r of g.roles) {
          const k = r + KEYSEP + o.name; if (!cm.domainPrivs.has(k)) cm.domainPrivs.set(k, { disclose: false, link: false });
          const e = cm.domainPrivs.get(k);
          for (const p of g.privs) {
            if (p.p === 'DISCLOSE') e.disclose = g.grant; else if (p.p === 'LINK') e.link = g.grant;
            else if (p.p === 'ALL') { e.disclose = g.grant; e.link = g.grant; } else throw wrong(p, 'match domain');
          }
        }
      } else {
        if (!su) throw denied(`permission denied for endpoint ${o.name}`, 'Only a superuser grants CONNECT.');
        if (!cm.endpoints.has(o.name)) throw sqlerr('42704', `endpoint "${o.name}" does not exist`);
        for (const p of g.privs) if (!['CONNECT', 'ALL'].includes(p.p)) throw wrong(p, 'endpoint');
        if (!cm.endpointGrants.has(o.name)) cm.endpointGrants.set(o.name, new Set());
        const s = cm.endpointGrants.get(o.name); for (const r of g.roles) (g.grant ? s.add(r) : s.delete(r));
      }
    });
    return { kind: 'command', tag };
  }

  execSetRole(a) {
    const s = this.s;
    let target = a.role;
    if (target === null || target === 'none' || target === 'default') target = s.sessionUser;
    const cat = this.cat();
    if (!cat.roles.has(target)) throw sqlerr('22023', `role "${target}" does not exist`);
    if (target !== s.sessionUser && !C.isSuper(cat, s.sessionUser)) throw denied(`permission denied to set role "${target}"`);
    if (!C.canConnect(cat, target, s.endpoint)) throw denied(`role "${target}" is not allowed to connect through endpoint "${s.endpoint}"`);
    s.role = target;
    return { kind: 'command', tag: 'SET' };
  }

  execRotateKey(ctx) {
    this.requireOperational('Key rotation is');
    this.requireSuper('rotate the master key');
    if (this.inTxn(this.s)) throw sqlerr('25001', 'ALTER SYSTEM ROTATE MASTER KEY cannot run inside a transaction block');
    this.ensureWriter();
    const old = this.keys, nk = new Keyring();
    let values = 0;
    for (const t of this.cat().tables.values()) {
      const prot = t.columns.filter((c) => c.domain);
      if (!prot.length) continue;
      this.s.txn.written.add(t.tid);
      const res = this.db.exec(`SELECT rowid, ${prot.map((c) => qi(c.name + CT)).join(', ')} FROM ${qi(t.name)}`);
      for (const row of res.length ? res[0].values : []) {
        const sets = [], args = [];
        prot.forEach((c, i) => {
          const ct = row[i + 1]; if (ct === null) return;
          const v = old.decrypt(c.domain, ct);
          if (v === null) throw sqlerr('XX001', `a protected value in ${t.name} could not be decrypted with the current key`);
          sets.push(`${qi(c.name)} = ?`, `${qi(c.name + CT)} = ?`); args.push(nk.indexEntry(c.domain, v), nk.encrypt(c.domain, v)); values++;
        });
        if (sets.length) this.db.run(`UPDATE ${qi(t.name)} SET ${sets.join(', ')} WHERE rowid = ?`, args.concat([row[0]]));
      }
    }
    this.keys = nk;
    this.s.pendingAudit.push({ kind: 'key_rotated', object: 'master key', domain: null, count: values });
    this.addNotes([`The master key was replaced: ${values} protected values were indexed and encrypted again with the new key, and the indexes were rebuilt. The old key no longer opens this data.`]);
    return { kind: 'command', tag: 'ALTER SYSTEM' };
  }

  /* --- SHOW */
  textRows(names, rows) { return { kind: 'rows', columns: names.map((n) => ({ name: n, tag: null })), rows, tag: `SHOW ${rows.length}` }; }

  execShow(a, s) {
    const w = a.words.map((x) => String(x));
    const W = w.map((x) => x.toUpperCase());
    const what = W[0] || '';
    const cat = this.cat();
    if (what === 'DEP' && W[1] === '.' && W[2] === 'PATH') return this.textRows(['dep.path'], [[s.path]]);
    if (what === 'DEP' && W[1] === 'NOTES') return this.textRows(['note'], s.notes.map((n) => [n]));
    if (what === 'PRIVACY' && W[1] === 'MATRIX') return this.privacyMatrix();
    if (what === 'ROLE' || what === 'CURRENT_USER') return this.textRows([what.toLowerCase()], [[s.role]]);
    if (what === 'SESSION_AUTHORIZATION' || (what === 'SESSION' && W[1] === 'AUTHORIZATION')) return this.textRows([w.join(' ').toLowerCase()], [[s.sessionUser]]);
    if (what === 'TRANSACTION') return this.textRows(['transaction_isolation'], [['repeatable read']]);
    if (what === 'AUDIT') {
      this.requireSuper('read the audit log');
      const n = w.map(Number).find((x) => Number.isInteger(x)) || 100;
      const recs = this.audit.slice(-n).reverse();
      return this.textRows(['seq', 'at', 'session_user', 'role', 'endpoint', 'path', 'kind', 'object', 'domain', 'count', 'detail'],
        recs.map((r) => [String(r.seq), r.at, r.session_user, r.role, r.endpoint, r.path, r.kind, r.object, r.domain, r.count > 0 ? String(r.count) : null, r.detail]));
    }
    const arg = w.slice(1).find((x) => !['FROM', 'LIMIT'].includes(x.toUpperCase()));
    const argName = arg === undefined ? null : arg.toLowerCase();
    switch (what) {
      case 'TABLES':
        return this.textRows(['table', 'protected_columns', 'rows', 'temporary'], sorted(cat.tables.keys()).map((n) => {
          const t = cat.tables.get(n);
          const cnt = this.readDb().exec(`SELECT count(*) FROM ${qi(n)}`)[0].values[0][0];
          return [n, t.columns.filter((c) => c.domain).map((c) => c.name).join(', '), String(cnt), t.temporary ? 'yes' : ''];
        }));
      case 'COLUMNS': {
        if (!argName) throw sqlerr('42601', 'SHOW COLUMNS FROM table');
        const t = cat.tables.get(argName); if (!t) throw sqlerr('42P01', `relation "${argName}" does not exist`);
        return this.textRows(['column', 'type', 'match_domain', 'primary_key', 'references', 'not_null', 'unique'], t.columns.map((c) => {
          const unique = t.indexes.some((i) => i.unique && i.columns.length === 1 && i.columns[0] === c.name);
          return [c.name, c.type, c.domain, c.pk ? 'yes' : '', c.references ? `${c.references.table} (${c.references.col})` : null, c.notNull ? 'yes' : '', unique ? 'yes' : ''];
        }));
      }
      case 'INDEXES': case 'INDEX': {
        if (argName && !cat.tables.has(argName)) throw sqlerr('42P01', `relation "${argName}" does not exist`);
        const rows = [];
        for (const n of sorted(cat.tables.keys())) {
          if (argName && n !== argName) continue;
          const t = cat.tables.get(n);
          for (const i of t.indexes) {
            const prot = i.columns.some((c) => (C.column(t, c) || {}).domain);
            rows.push([n, i.name, i.columns.join(', '), i.primary ? 'primary key' : i.unique ? 'unique' : '', !i.unique ? '' : i.enforced ? 'yes' : 'no (duplicate keys)', i.auto ? 'engine' : 'CREATE INDEX', prot ? 'blind index entries' : 'values']);
          }
        }
        return this.textRows(['table', 'index', 'columns', 'kind', 'enforced', 'made_by', 'keys'], rows);
      }
      case 'FUNCTIONS':
        return this.textRows(['function', 'discloses', 'links', 'owner', 'masked_path', 'comment', 'executable_by', 'body'], cat.functionOrder.map((n) => cat.functions.get(n)).filter(Boolean).map((f) => [
          `${f.name}(${f.params.map((p) => p.type).join(', ')})`, f.discloses.join(', '), f.links.join(', '), f.owner, f.maskedOk ? 'yes' : 'no', f.comment, sorted(cat.fnExecute.get(f.name) || []).join(', '), f.body]));
      case 'ROLES':
        return this.textRows(['role', 'superuser', 'login', 'endpoints', 'match_limit'], sorted(cat.roles.keys()).map((n) => {
          const r = cat.roles.get(n);
          const eps = sorted(cat.endpoints.keys()).filter((e) => C.canConnect(cat, n, e));
          const ml = r.matchLimit.kind === 'limit' ? describeLimit(r.matchLimit) : r.matchLimit.kind === 'none' ? 'none' : r.superuser ? '' : this.defaultLimit ? `${describeLimit(this.defaultLimit)} (server default)` : '';
          return [n, r.superuser ? 'yes' : '', r.login ? 'yes' : '', eps.join(', '), ml];
        }));
      case 'GRANTS': return this.textRows(['grantee', 'privilege'], this.grantLines(cat));
      case 'ENDPOINTS': return this.textRows(['endpoint', 'path'], sorted(cat.endpoints.keys()).map((e) => [e, cat.endpoints.get(e)]));
      case 'STORAGE': {
        this.requireSuper('see the stored form of a table');
        if (!argName) throw sqlerr('42601', 'SHOW STORAGE table [LIMIT n]');
        if (!cat.tables.has(argName)) throw sqlerr('42P01', `relation "${argName}" does not exist`);
        const n = w.map(Number).find((x) => Number.isInteger(x)) || 1000;
        const cols = physColumns(cat.tables.get(argName));
        const res = this.readDb().exec(`SELECT ${cols.map(qi).join(', ')} FROM ${qi(argName)} ORDER BY rowid LIMIT ${n}`);
        return { kind: 'rows', columns: cols.map((c) => ({ name: c, tag: null })), rows: res.length ? res[0].values : [], tag: 'SHOW' };
      }
      default: throw sqlerr('42601', `unrecognized SHOW ${what}`);
    }
  }

  grantLines(cat) {
    const v = [];
    for (const k of sorted(cat.tablePrivs.keys())) {
      const [role, table] = k.split(KEYSEP); const p = cat.tablePrivs.get(k); const d = [];
      if (p.selectAll) d.push('SELECT'); else if (p.selectCols.size) d.push(`SELECT (${sorted(p.selectCols).join(', ')})`);
      if (p.insert) d.push('INSERT');
      if (p.updateAll) d.push('UPDATE'); else if (p.updateCols.size) d.push(`UPDATE (${sorted(p.updateCols).join(', ')})`);
      if (p.delete) d.push('DELETE');
      if (d.length) v.push([role, `${d.join(', ')} ON ${table}`]);
    }
    for (const k of sorted(cat.domainPrivs.keys())) {
      const [role, d] = k.split(KEYSEP); const e = cat.domainPrivs.get(k);
      const kinds = [e.disclose ? 'DISCLOSE' : null, e.link ? 'LINK' : null].filter(Boolean);
      if (kinds.length) v.push([role, `${kinds.join(', ')} ON MATCH DOMAIN ${d}`]);
    }
    for (const f of sorted(cat.fnExecute.keys())) for (const r of sorted(cat.fnExecute.get(f))) v.push([r, `EXECUTE ON FUNCTION ${f}`]);
    for (const e of sorted(cat.endpointGrants.keys())) for (const r of sorted(cat.endpointGrants.get(e))) v.push([r, `CONNECT ON ENDPOINT ${e}`]);
    return v;
  }

  /* Who may reach each protected column, and how. A role that is not a superuser sees its own rows. */
  privacyMatrix() {
    const cat = this.cat(), me = this.role(), su = C.isSuper(cat, me);
    const rows = [];
    for (const tn of sorted(cat.tables.keys())) {
      const t = cat.tables.get(tn);
      for (const c of t.columns.filter((x) => x.domain)) {
        for (const rn of sorted(cat.roles.keys())) {
          if (!su && rn !== me) continue;
          const r = cat.roles.get(rn);
          const paths = sorted(cat.endpoints.keys()).filter((e) => r.login && C.canConnect(cat, rn, e)).map((e) => `${e} (${cat.endpoints.get(e)})`);
          const via = (k) => sorted(cat.functions.keys()).map((n) => cat.functions.get(n)).filter((f) => (k === 'd' ? f.discloses : f.links).includes(c.domain) && C.canExecute(cat, rn, f)).map((f) => `${f.name}()`).join(', ');
          rows.push([c.domain, `${tn}.${c.name}`, rn, paths.join(', '), C.canSelect(cat, rn, tn, c.name) ? 'yes' : 'no', via('d'), via('l')]);
        }
      }
    }
    return this.textRows(['match_domain', 'column', 'role', 'paths', 'may_match', 'discloses_via', 'links_via'], rows);
  }

  /* --- inspection, for the playground */
  describe(s) {
    const prev = this.s; this.s = s || null;
    try {
      const cat = this.cat(); const db = s ? this.readDb() : this.db;
      return {
        tables: sorted(cat.tables.keys()).map((n) => {
          const t = cat.tables.get(n);
          let rows = null; try { rows = db.exec(`SELECT count(*) FROM ${qi(n)}`)[0].values[0][0]; } catch (e) { /* not visible */ }
          return { name: n, temporary: t.temporary, rows, columns: t.columns.map((c) => ({ name: c.name, type: c.type, domain: c.domain, pk: c.pk, notNull: c.notNull, references: c.references, unique: t.indexes.some((i) => i.unique && i.columns.length === 1 && i.columns[0] === c.name) })), indexes: t.indexes.map((i) => Object.assign({}, i)) };
        }),
        functions: cat.functionOrder.map((n) => cat.functions.get(n)).filter(Boolean).map((f) => ({ name: f.name, signature: `${f.name}(${f.params.map((p) => p.type).join(', ')})`, params: f.params.map((p) => ({ name: p.name, type: p.type })), returns: f.returns.map((r) => ({ name: r.name, type: r.type })), discloses: f.discloses, links: f.links, owner: f.owner, comment: f.comment, maskedOk: f.maskedOk, executableBy: sorted(cat.fnExecute.get(f.name) || []), body: f.body })),
        roles: sorted(cat.roles.keys()).map((n) => { const r = cat.roles.get(n); return { name: n, superuser: r.superuser, login: r.login, endpoints: sorted(cat.endpoints.keys()).filter((e) => C.canConnect(cat, n, e)), matchLimit: r.matchLimit.kind === 'limit' ? describeLimit(r.matchLimit) : r.matchLimit.kind }; }),
        endpoints: sorted(cat.endpoints.keys()).map((e) => ({ name: e, path: cat.endpoints.get(e) })),
        grants: this.grantLines(cat).map(([grantee, privilege]) => ({ grantee, privilege })),
        domains: sorted(C.domains(cat))
      };
    } finally { this.s = prev; }
  }
  storageView(table, s) {
    const prev = this.s; this.s = s || null;
    try {
      const t = this.cat().tables.get(table); if (!t) return null;
      const cols = physColumns(t); const db = s ? this.readDb() : this.db;
      const res = db.exec(`SELECT ${cols.map(qi).join(', ')} FROM ${qi(table)} ORDER BY rowid`);
      return { columns: cols, rows: res.length ? res[0].values : [] };
    } finally { this.s = prev; }
  }
}

function refersToColumns(e) {
  let found = false;
  const walk = (x) => {
    if (!x || typeof x !== 'object' || found) return;
    if (x.type === 'col' || x.type === 'scalarsub' || x.type === 'exists' || x.type === 'insub') { found = true; return; }
    for (const k of Object.keys(x)) { if (k === 'tok') continue; const v = x[k]; if (Array.isArray(v)) v.forEach(walk); else if (v && typeof v === 'object') walk(v); }
  };
  walk(e);
  return found;
}

function toSqlError(e) {
  if (e instanceof SqlError) return e;
  const m = String((e && e.message) || e);
  let r;
  if ((r = /no such function: (\w+)/.exec(m))) return sqlerr('42883', `function ${r[1]}() does not exist`);
  if (/no such column/.test(m)) return sqlerr('42703', m.replace(/^no such column: /, 'column ') + ' does not exist');
  return sqlerr('XX000', m);
}
function errObj(e) {
  const x = toSqlError(e);
  return { message: x.message, code: x.code, sqlstate: x.sqlstate, hint: x.hint };
}

/* ------------------------------------------------------------------ public API */

class Connection {
  constructor(engine, session) { this.engine = engine; this.session = session; }
  get role() { return this.session.role; }
  get sessionUser() { return this.session.sessionUser; }
  get endpoint() { return this.session.endpoint; }
  get path() { return this.session.path; }
  get inTransaction() { return !!(this.session.txn && this.session.txn.explicit); }
  get transactionFailed() { return !!(this.session.txn && this.session.txn.failed); }
  /* Runs a script (a string) or one statement from DepSQL.split(). Returns one result per statement. */
  exec(sql, params) {
    const e = this.engine;
    params = (params || []).map((v) => (v === undefined || v === '' ? null : v));
    const list = typeof sql === 'string' ? (() => { try { return splitStatements(sql); } catch (x) { return x; } })() : [sql];
    if (list instanceof Error) return [{ ok: false, text: String(sql).trim(), notes: [], error: errObj(list), session: e.sessionInfo(this.session) }];
    const out = [];
    for (const st of list) {
      const r = e.runOne(this.session, st, params);
      if (r.newSession) { this.session = r.newSession; delete r.newSession; }
      out.push(r);
    }
    return out;
  }
  /* Ends the connection: an open transaction is rolled back. */
  close() { this.engine.closeSession(this.session); }
}

class Server {
  /* new DepSQL.Server(SQL, { setup, matchLimit })
     SQL         the object returned by initSqlJs()
     setup       a DEP SQL script (string, or function returning one) run as admin when the server starts
     matchLimit  the default match limit for roles that are not superusers, such as '1000/minute' */
  constructor(SQL, opts) {
    this.engine = new Engine(SQL, Object.assign({}, opts || {}));
    this.setupResults = this.engine.setupResults;
  }
  connect(role, endpoint) { return new Connection(this.engine, this.engine.openSession(role, endpoint || 'operational')); }
  describe(conn) { return this.engine.describe(conn ? conn.session : null); }
  storage(table, conn) { return this.engine.storageView(table, conn ? conn.session : null); }
  get auditLog() { return this.engine.audit.slice(); }
  get sqlite() { return this.engine.db; }
  reset() { this.engine.reset(); this.setupResults = this.engine.setupResults; }
}

const api = {
  version: VERSION,
  Server, Connection, SqlError, SQLSTATE,
  split: splitStatements,
  tokenize,
  parse: (sql) => splitStatements(sql).map((s) => (s.toks[0].t === 'meta' ? { type: 'meta', text: s.text } : new Parser(s.toks).statement()))
};
if (typeof module !== 'undefined' && module.exports) module.exports = api;
global.DepSQL = api;
})(typeof window !== 'undefined' ? window : globalThis);
