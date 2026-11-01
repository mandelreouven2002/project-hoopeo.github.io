/* DEP SQL for CodeMirror: PostgreSQL-flavoured SQL plus the DEP keywords. */
(function () {
  'use strict';
  const words = (s) => s.split(' ').reduce((o, w) => { o[w] = true; return o; }, {});
  const base = 'alter all and any as asc between by case cast check column comment constraint create cross current_date current_time current_timestamp default delete desc distinct drop else end except exists explain false for foreign from full function grant group having if in inner insert intersect into is join key language left like limit not null offset on or order outer primary references replace reset returns revoke right role select set show table then to true union update user using values view when where with';
  const dep = 'protected match domain discloses links reveal endpoint mode operational masked_read disclose link execute connect login nologin superuser nosuperuser password storage tables columns functions roles grants endpoints';
  CodeMirror.defineMIME('text/x-depsql', {
    name: 'sql',
    client: {},
    keywords: words(base + ' ' + dep),
    builtin: words('bigint int integer smallint text varchar char boolean bool date timestamp timestamptz double precision float real numeric decimal'),
    atoms: words('false true null'),
    operatorChars: /^[*+\-%<>!=&|^\/#@?~]/,
    dateSQL: words('date time timestamp'),
    support: words('ODBCdotTable decimallessFloat zerolessFloat binaryNumber hexNumber nCharCast charsetCast'),
    hooks: {
      '$': function (stream) {
        // $$ ... $$ function bodies, and $1 parameters
        if (stream.match(/^\$\$/)) { return 'string-2'; }
        if (stream.match(/^\d+/)) { return 'variable-2'; }
        return null;
      }
    }
  });
})();
