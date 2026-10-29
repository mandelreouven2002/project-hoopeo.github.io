'use strict';
const fs=require('node:fs');const vm=require('node:vm');const assert=require('node:assert/strict');
const hljs=require('../assets/vendor/highlight/highlight.min.js');global.hljs=hljs;require('../assets/vendor/highlight/sql.min.js');
const context={window:{hljs},hljs,document:{querySelectorAll:()=>[],getElementById:()=>null}};
vm.runInNewContext(fs.readFileSync(require('node:path').join(__dirname,'../assets/js/code-highlighting.js'),'utf8'),context);
const highlight=context.window.HoopeoCode.sql;
for(const keyword of ['SELECT','FROM','WHERE','PROTECTED','DISCLOSES','LINKS','REVEAL']) {
  assert.match(highlight(keyword+' x'),new RegExp('class="hljs-keyword">'+keyword+'</span>'),keyword+' must be a colored keyword');
}
assert.match(highlight("SELECT '<script>' -- a comment"),/hljs-string/);
assert.match(highlight("SELECT '<script>' -- a comment"),/hljs-comment/);
assert.ok(!highlight("SELECT '<script>'").includes('<script>'),'input must be escaped, not executable markup');
console.log('PASS: SQL and DEP keyword colors, strings, comments and HTML escaping.');
