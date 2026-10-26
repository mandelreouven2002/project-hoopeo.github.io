"""The terms of the glossary that site/build.py links wherever a page uses them.

Each entry: the heading of the term in docs/reference/glossary.md, and the ways it is written in text.
A variant is a regular expression; one written in capitals (REVEAL, MASKED_READ) matches only in capitals,
the others in any case. The build links the first occurrence of each term on a page, in body text only (not
in headings, code, links, or the glossary itself), and gives the link the one-line definition from the glossary
(the bold line under the heading) in data-def, which the page script shows as a popover.
"""
import re

TERMS = [
    ('Protected column', [r'protected columns?', r'PROTECTED columns?']),
    ('Match domain', [r'match domains?', r'MATCH DOMAINS?']),
    ('Blind index', [r'blind index(?:es)?', r'index entr(?:y|ies)']),
    ('Ciphertext', [r'ciphertexts?']),
    ('Master key', [r'master key']),
    ('Endpoint', [r'endpoints?']),
    ('Access path', [r'access paths?']),
    ('Masked read path', [r'masked read path', r'MASKED_READ']),
    ('Operational path', [r'operational path', r'OPERATIONAL']),
    ('Matching', [r'matching']),
    ('Anchor', [r'unanchored', r'anchored']),
    ('Linking', [r'linking', r'LINKS']),
    ('Disclosure', [r'disclosures?', r'disclosed', r'disclosing', r'DISCLOSES']),
    ('Terminal rule', [r'terminal rule']),
    ('REVEAL', [r'REVEAL']),
    ('Declared function', [r'declared functions?']),
    ('DEP note', [r'DEP notes?']),
    ('DEP error', [r'DEP errors?']),
    ('Audit log', [r'audit log']),
    ('Match limit', [r'match limits?']),
    ('Privacy matrix', [r'privacy matrix']),
]


def slug(name):
    return re.sub(r'[^\w\s-]', '', name.strip().lower()).replace(' ', '-')


def definitions(glossary_text):
    """{heading: one-line definition} from the glossary Markdown."""
    out = {}
    for m in re.finditer(r'^### (.+?)\n\n\*\*(.+?)\*\*', glossary_text, re.M):
        out[m.group(1).strip()] = m.group(2).strip()
    return out


def _pattern(variants):
    parts = []
    for v in variants:
        if v.upper() == v.replace('\\', '').replace('?', '').upper() and re.search(r'[A-Z]{3}', v):
            parts.append('(?-i:%s)' % v)
        else:
            parts.append('(?i:%s)' % v)
    return re.compile(r'\b(?:%s)\b' % '|'.join(parts))


# longest variants first would be ideal; the terms are matched together, leftmost match wins, longer first on ties
ALL = []
for name, variants in TERMS:
    for v in variants:
        ALL.append((name, v))
