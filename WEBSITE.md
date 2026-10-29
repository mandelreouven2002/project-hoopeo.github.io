# Project Hoopeo website

This repository contains the Project Hoopeo website and documentation by Reuven Mandelovich. Documentation is licensed under CC BY 4.0; DEP.js, the playground and the starter examples are licensed under Apache 2.0. See LICENSING.md and THIRD-PARTY.md.

## Documentation structure

The primary documentation is one book in docs/book, in the approved eight-chapter sequence. Chapters 1 to 4, 7 and 8 develop the original DEP design proposal; chapters 5 and 6 consolidate the runnable integration and operations material. The original proposal and old guides are source pointers, not parallel tutorials.

site/book.json defines chapter routes and legacy fragment destinations. The build emits static redirects with JavaScript fragment mapping and a no-JavaScript fallback. Search and sitemap include canonical pages only. Reference has six destinations; notes and errors share one page. Playground help is embedded in the playground.

Privacy as Code remains a separate beginner introduction, linked from the main menu and leading to chapter 7. Home, Learn DEP, Privacy as Code, Playground, Install and GitHub are the six main navigation links.

site/illustrations.json maps all 30 existing illustration families to their reader locations. All light, dark and narrow variants remain available. The 16 original Mermaid definitions are actual diagrams using the local MIT-licensed Mermaid Tiny 12.2.0 bundle, loaded only when needed. The original architecture image and interactive architecture are retained.

Current syntax replaces conflicting proposal examples for protected declarations, equality/IN lookups, joins and refused operations. Qualified access and suffix disclosure are marked as future design. SHOW FUNCTIONS replaces the unimplemented catalog interface in practical examples. The reported native disclosed-sort gap remains documented; the native Engine was not changed or run.

## Local preview

The downloadable archive includes a complete static build in `dist/`.

```sh
python3 -m http.server 8000 --directory dist
```

Open http://localhost:8000/. Use a local HTTP server, not a file:// URL, because WebAssembly and the documentation search load files.

## Rebuild from source

Requires Python 3.12+ and Node.js 22+.

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install -r requirements-site.txt
npm ci --prefix playground
BASE_URL=http://localhost:8000/ python site/build.py --check
```

`BASE_URL` is the hosting origin and path. Build specifically for the deployment destination, then check its internal links:

```sh
BASE_URL=https://mandelreouven2002.github.io/project-hoopeo.github.io/ python site/build.py --check
```

Upload the contents of `dist/` to the existing GitHub Pages site. A localhost build uses `/` paths and must be rebuilt with the command above before deployment to the repository subpath. The source defaults to the existing GitHub Pages address when `BASE_URL` is unset.

The downloadable package contains `dist/` for localhost preview and `github-pages/` built and checked for the existing public address. These are two build targets of the same website, not two published sites.

## Verification

```sh
npm test --prefix playground
node playground/test-docs.js
node playground/test-book.js
node site/tools/test-highlighting.cjs
BASE_URL=http://localhost:8000/ python site/build.py --check
python site/tools/test-book.py
```

The first command runs 38 upstream simulator checks. The second initializes the downloadable tutorial schema and checks its lookup, grants, joins, analytics, disclosure and terminal-rule behavior against DEP.js. The highlighting check verifies the local SQL/DEP grammar. The last builds the complete site and checks internal links and fragments.

The native DEP Engine binary is not included or changed. Run `starter/python/verify.py` against a running Engine to verify the native integration. The simulator follows the specified terminal rule. A project review reports a disclosed-sort enforcement gap in native 0.5.1; native 0.5.2 has not been retested here. See docs/book/06-server.md#known-difference-sorting-disclosed-values. A passing simulator test is not evidence of native Engine behavior.

## Verification scope

The book checks verify eight canonical chapters, unique search entries, legacy fragment targets, all 16 diagram definitions, all illustration families and the absence of parallel tutorial bodies in old sources. The JavaScript book test runs the actual four function definitions taught in chapter 3 on the clinic simulator. Native integration remains separate.

The Mermaid bundle is preserved byte-for-byte from its npm package. Its upstream whitespace inside template strings is retained. Authored files are checked with git diff --check separately from that vendor file.
