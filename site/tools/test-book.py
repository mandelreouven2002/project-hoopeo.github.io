"""Check the reader's navigation, old deep links and preservation of illustrations.

Run after site/build.py --check, using the same Python environment.
"""
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'dist'
CONFIG = json.loads((ROOT / 'site/book.json').read_text())


class BookContract(unittest.TestCase):
    def setUp(self):
        self.pages = {p.relative_to(OUT).as_posix().removesuffix('index.html'): p.read_text()
                      for p in OUT.rglob('index.html')}
        self.ids = {path: set(re.findall(r'\bid="([^"]+)"', text)) for path, text in self.pages.items()}

    def test_eight_chapters_and_one_search_entry_per_page(self):
        chapters = CONFIG['chapters']
        self.assertEqual(len(chapters), 8)
        search = json.loads((OUT / 'search-index.json').read_text())
        self.assertEqual(len(search), len({row['url'] for row in search}))
        for n, chapter in enumerate(chapters, 1):
            page = self.pages[chapter['url']]
            self.assertIn(f'Chapter {n} of 8', page)
            self.assertIn('<summary>Reference</summary>', page)
            self.assertEqual(page.count('class="chapter-number"'), 16)  # desktop and mobile
            self.assertEqual(sum(row['title'] == chapter['title'] for row in search), 1)
        self.assertNotIn('DEP SQL in five minutes', {row['title'] for row in search})

    def test_every_legacy_fragment_has_a_real_destination(self):
        for source, item in CONFIG['redirects'].items():
            for target in [item['target'], *item['fragments'].values()]:
                page, _, fragment = target.partition('#')
                with self.subTest(source=source, target=target):
                    self.assertIn(page, self.ids)
                    if fragment:
                        self.assertIn(fragment, self.ids[page])

    def test_all_sixteen_original_diagrams_are_renderable_figures(self):
        pages = [self.pages[ch['url']] for ch in CONFIG['chapters']]
        self.assertEqual(sum(page.count('class="mermaid"') for page in pages), 16)
        for page in pages:
            self.assertNotIn('class="language-mermaid"', page)

    def test_all_existing_illustration_families_have_a_reader_location(self):
        inventory = json.loads((ROOT / 'site/illustrations.json').read_text())
        actual = {re.sub(r'(?:-narrow)?(?:-dark)?$', '', p.with_suffix('').as_posix())
                  for p in (ROOT / 'docs/images').rglob('*.png')}
        mapped = {(ROOT / item['asset']).with_suffix('').as_posix() for item in inventory}
        self.assertEqual(actual, mapped)
        for item in inventory:
            filename = re.escape(Path(item['asset']).name)
            self.assertRegex((ROOT / item['source']).read_text(), r'(?:/|["(])' + filename + r'(?=["\s)])')

    def test_proposal_and_old_guides_are_pointers_not_parallel_tutorials(self):
        for source in CONFIG['redirects']:
            if source.startswith('docs/guides/') or source == 'docs/design/01-dep-concept.md':
                self.assertLess(len((ROOT / source).read_text()), 500)


if __name__ == '__main__':
    unittest.main()
