"""Offline regressions for documentation link validation."""
from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-markdown-links.py")
spec = importlib.util.spec_from_file_location("check_markdown_links", SCRIPT)
links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(links)


class MarkdownLinkChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "docs" / "sub").mkdir(parents=True)
        (self.root / "README.md").write_text("# Home\n", encoding="utf-8")
        (self.root / "docs" / "a.md").write_text("ok", encoding="utf-8")
        (self.root / "docs" / "sub" / "b.md").write_text("ok", encoding="utf-8")
        (self.root / "docs" / "image with space.png").write_bytes(b"png")

    def check(self, content):
        doc = self.root / "docs" / "sub" / "check.md"
        doc.write_text(content, encoding="utf-8")
        return links.find_broken_links(self.root, [doc])

    def test_nested_relative_links_queries_fragments_and_images(self):
        self.assertEqual(self.check(
            '[guide](../a.md#section) ![img](<../image with space.png>)\n'
            '[other](b.md?raw=1#top) [encoded](../image%20with%20space.png)'), [])

    def test_missing_target_reports_file_and_line(self):
        self.assertEqual(self.check('# Title\n[missing](../no-such-file.md)\n'), [
            'docs/sub/check.md:2: missing local link target: ../no-such-file.md'])

    def test_reference_links_full_collapsed_and_shortcut(self):
        self.assertEqual(self.check(
            '[first][guide] [guide][] [guide]\n\n[guide]: <../a.md> "Documentation"\n'), [])
        self.assertEqual(self.check('[bad][ref]\n[ref]: ../missing.md\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_empty_inline_link_and_image_labels_are_checked(self):
        self.assertEqual(self.check('[](../missing.md)\n![](../missing.png)\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md',
            'docs/sub/check.md:2: missing local link target: ../missing.png'])
        self.assertEqual(self.check('[](../a.md) ![](../image%20with%20space.png)'), [])

    def test_shortcut_image_reference_checks_target(self):
        self.assertEqual(self.check('![diagram]\n[diagram]: ../missing.png\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.png'])
        self.assertEqual(self.check('![diagram]\n[diagram]: <../image with space.png>\n'), [])

    def test_multiline_inline_code_does_not_report_links(self):
        self.assertEqual(self.check(
            'This is `example\n[hidden](../missing.md)`\n[real](../a.md)\n'), [])
        self.assertEqual(self.check(
            'Start ``example\n![hidden](../missing.png)``\n'), [])

    def test_multiline_inline_code_preserves_error_line_numbers(self):
        self.assertEqual(self.check(
            '`example\n[hidden](../missing.md)`\n[broken](../absent.md)\n'), [
            'docs/sub/check.md:3: missing local link target: ../absent.md'])

    def test_unmatched_backticks_across_paragraphs_do_not_hide_broken_links(self):
        self.assertEqual(self.check(
            'An unmatched ` delimiter.\n\n[broken](../missing.md)\n\n'
            'Another unmatched ` delimiter.\n'), [
            'docs/sub/check.md:3: missing local link target: ../missing.md'])

    def test_code_spans_do_not_cross_heading_or_list_boundaries(self):
        self.assertEqual(self.check(
            'Unmatched ` delimiter\n# A heading\n[broken](../missing.md)\n'), [
            'docs/sub/check.md:3: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            'Unmatched ` delimiter\n- [broken](../missing.md)\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            'Unmatched ` delimiter\n> [broken](../missing.md)\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])

    def test_code_spans_within_blockquotes_and_list_items(self):
        self.assertEqual(self.check(
            '> `example\n> [hidden](../missing.md)`\n'), [])
        self.assertEqual(self.check(
            '- `example\n  [hidden](../missing.md)`\n'), [])

    def test_escaped_backticks_do_not_hide_broken_links(self):
        self.assertEqual(self.check(
            r'Escaped \` [broken](../missing.md) \`' + '\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_code_delimiter_runs_must_match_exactly(self):
        self.assertEqual(self.check(
            '`unmatched\n[broken](../missing.md)\n``also unmatched\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])

    def test_fenced_code_remains_ignored_after_block_scoping(self):
        self.assertEqual(self.check(
            '```md\n[hidden](../missing.md)\n```\n'
            '[visible](../missing.md)\n'), [
            'docs/sub/check.md:4: missing local link target: ../missing.md'])

    def test_paragraph_code_spans_can_still_contain_newlines(self):
        self.assertEqual(self.check(
            'Intro ``example\n[hidden](../missing.md)``\n'
            '[visible](../missing.md)\n'), [
            'docs/sub/check.md:3: missing local link target: ../missing.md'])

    def test_external_urls_fragments_and_root_relative_are_skipped(self):
        self.assertEqual(self.check(
            '[web](https://example.org/x) [email](mailto:help@example.org)\n'
            '[fragment](#intro) [root](/docs) <img src="https://example.org/x.png">'), [])

    def test_code_examples_and_html_comments_are_ignored(self):
        self.assertEqual(self.check(
            '`[inline](missing.md)`\n```markdown\n[hidden](missing.md)\n```\n'
            '~~~\n![also hidden](missing.png)\n~~~\n'
            '<!-- [comment](missing.md) -->\n'), [])

    def test_html_relative_links_and_parentheses_in_destinations(self):
        self.assertEqual(self.check(
            '<a href="../a.md">guide</a> <img src="../image%20with%20space.png">\n'
            '[nested](../image%20with%20space.png) [p](b.md)'), [])
        self.assertEqual(self.check('<a href="../missing.html">broken</a>'), [
            'docs/sub/check.md:1: missing local link target: ../missing.html'])

    def test_relative_path_cannot_escape_checkout(self):
        self.assertEqual(self.check('[outside](../../../secret.txt)'), [
            'docs/sub/check.md:1: missing local link target: ../../../secret.txt'])


if __name__ == "__main__":
    unittest.main()
