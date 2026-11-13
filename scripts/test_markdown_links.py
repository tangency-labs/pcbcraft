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

    def test_review_closing_backtick_after_backslash(self):
        self.assertEqual(self.check(
            '`[hidden](../missing.md)\\`\n[visible](../absent.md)\n'), [
            'docs/sub/check.md:2: missing local link target: ../absent.md'])

    def test_review_ordered_list_other_than_one_continues_paragraph(self):
        self.assertEqual(self.check(
            'Intro `example\n2. [hidden](../missing.md)`\n'), [])
        self.assertEqual(self.check(
            'Intro `example\n20) [hidden](../missing.md)`\n'), [])
        self.assertEqual(self.check(
            'Intro `example\n1. [visible](../missing.md)`\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])

    def test_review_blockquote_lazy_continuation(self):
        self.assertEqual(self.check(
            '> `example\n[hidden](../missing.md)`\n'), [])
        self.assertEqual(self.check(
            '> > `example\n[hidden](../missing.md)`\n'), [])

    def test_review_blank_line_blocks_quote_continuation(self):
        self.assertEqual(self.check(
            '> `example\n\n[visible](../missing.md)`\n'), [
            'docs/sub/check.md:3: missing local link target: ../missing.md'])

    def test_review_nested_blockquote_and_list_are_separate(self):
        self.assertEqual(self.check(
            '> `example\n- [visible](../missing.md)`\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])

    def test_valid_escaped_backtick_opener_is_not_code(self):
        self.assertEqual(self.check(
            r'\`[visible](../missing.md)\`' + '\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_inline_parenthesized_title_and_escaped_title_quotes(self):
        self.assertEqual(self.check(
            '[p](../missing.md (description))\n'
            r'[q](../missing.md "escaped \"quote\" value")' + '\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md',
            'docs/sub/check.md:2: missing local link target: ../missing.md'])

    def test_escaped_parentheses_in_local_destinations(self):
        (self.root / "docs" / "diagram(one).md").write_text("ok", encoding="utf-8")
        self.assertEqual(self.check(
            r'[valid](../diagram\(one\).md)' + '\n'), [])
        self.assertEqual(self.check(
            r'[missing](../not-here\(one\).md)' + '\n'), [
            r'docs/sub/check.md:1: missing local link target: ../not-here\(one\).md'])

    def test_html_entities_in_link_destinations(self):
        (self.root / "docs" / "a&b.md").write_text("ok", encoding="utf-8")
        self.assertEqual(self.check(
            '[guide](../a&amp;b.md) <a href="../a&amp;b.md">ok</a>\n'), [])

    def test_escaped_closing_parenthesis_in_destination(self):
        (self.root / "docs" / "diagram)one.md").write_text("ok", encoding="utf-8")
        self.assertEqual(self.check(r'[ok](../diagram\)one.md)' + '\n'), [])

    def test_heading_in_blockquote_does_not_lazy_continue(self):
        self.assertEqual(self.check(
            '> # Heading `\n[broken](../missing.md)`\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            '> ## Heading `\n[broken](../missing.md)`\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            '> Paragraph `\n[hidden](../missing.md)`\n'), [])

    def test_inline_missing_link_title_on_next_line(self):
        self.assertEqual(self.check(
            '[broken](../missing.md\n  "Description")\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            '[broken](../missing.md\n)\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            '[broken](<../missing.md>\n  (Description))\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_multiline_valid_link_and_no_duplicate_reference(self):
        self.assertEqual(self.check(
            '[ok](../a.md\n  "Description") and [another](b.md)\n'), [])
        self.assertEqual(self.check(
            '[bad](../missing.md\n  "Label") [unrelated][guide]\n'
            '[guide]: ../a.md\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_double_blank_line_does_not_form_inline_link(self):
        self.assertEqual(self.check(
            '[not a link](../missing.md\n\n "Title")\n'), [])

    def test_heading_does_not_extend_link_into_next_block(self):
        self.assertEqual(self.check(
            '# [not a link](../missing.md\n  "Description")\n'), [])
        self.assertEqual(self.check(
            '> # [not a link](../missing.md\n  "Description")\n'), [])
        self.assertEqual(self.check(
            '[broken](../missing.md\n  "Description")\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_semicolonless_entities_in_markdown_destinations_are_literal(self):
        (self.root / "docs" / "a&ampcopy.md").write_text("ok", encoding="utf-8")
        self.assertEqual(self.check('[ok](../a&ampcopy.md)\n'), [])
        self.assertEqual(self.check('[missing](../absent&ampcopy.md)\n'), [
            'docs/sub/check.md:1: missing local link target: ../absent&ampcopy.md'])

    def test_html_attributes_remain_html5_decoded(self):
        (self.root / "docs" / "a&copy.md").write_text("ok", encoding="utf-8")
        self.assertEqual(self.check('<a href="../a&amp;copy.md">link</a>\n'), [])

    def test_multiple_multiline_links_preserve_line_numbers(self):
        self.assertEqual(self.check(
            '[first](../missing.md\n  "First")\n'
            '[second](../absent.png\n  "Second")\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md',
            'docs/sub/check.md:3: missing local link target: ../absent.png'])

    def test_block_boundary_inside_multiline_link_title(self):
        # CommonMark headings and list items terminate the inline block, even
        # when an open quotation-marked link title preceded them.
        self.assertEqual(self.check(
            '[text](../missing.md "Title\n# Heading\nend")\n'), [])
        self.assertEqual(self.check(
            '[text](../missing.md "Title\n- next item\nend")\n'), [])
        self.assertEqual(self.check(
            '[text](../missing.md "Title\n> next quote\nend")\n'), [])

    def test_multiline_link_inside_blockquote(self):
        self.assertEqual(self.check(
            '> [broken](../missing.md\n>   "Description")\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])
        self.assertEqual(self.check(
            '> [valid](../a.md\n>   "Description")\n'), [])
        self.assertEqual(self.check(
            '> First [broken](../missing.md\n> )\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_multiline_quoted_link_after_unquoted_paragraph(self):
        self.assertEqual(self.check(
            '[first](../missing.md "title\n> [second](../missing.md\n> "ok")\n'), [
            'docs/sub/check.md:2: missing local link target: ../missing.md'])

    def test_multiline_link_inside_list_item(self):
        self.assertEqual(self.check(
            '- [broken](../missing.md\n  "Description")\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_nested_or_escaped_brackets_inside_inline_link_label(self):
        self.assertEqual(self.check(
            '[link [nested]](../missing.md)\n'
            r'[link \[bracket\]](../absent.md)' + '\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md',
            'docs/sub/check.md:2: missing local link target: ../absent.md'])
        self.assertEqual(self.check(
            r'\[not a link](../missing.md)' + '\n'), [])
        self.assertEqual(self.check(
            '[nested [deeper [value]]](../missing.md)'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

    def test_markdown_like_text_inside_html_tag_attributes_is_not_a_link(self):
        self.assertEqual(self.check(
            '<span title="[not a link](../missing.md)">ok</span>\n'), [])
        self.assertEqual(self.check(
            '<a href="../missing.md" title="[not a link](../also-missing.md)">x</a>\n'), [
            'docs/sub/check.md:1: missing local link target: ../missing.md'])

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
