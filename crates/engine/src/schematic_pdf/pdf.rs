//! Single-page PDF encoding. The writer receives points and text, not project data.

use std::{collections::BTreeMap, fmt::Write};

use crate::{board::silkscreen::stroke_glyph, library_graphics::Fill, schematic::TextAlign};

use super::PdfError;

// Type 3 uses one byte per character. The same page can use several font dictionaries.
const CODES: usize = 255;
const GLYPH_STROKE: f64 = 70.;

pub(super) struct Page {
    size: [f64; 2],
    ink: String,
    codes: BTreeMap<char, usize>,
    glyphs: Vec<Glyph>,
}

struct Glyph {
    unicode: char,
    width: f64,
    bounds: [f64; 4],
    program: String,
}

impl Page {
    /// Start in PDF points with a paper-sized clip, black ink and round line ends and joins.
    pub(super) fn new([width, height]: [f64; 2]) -> Self {
        Self { size: [width, height], ink: format!("q\n0 0 {width:.5} {height:.5} re W n\n1 J 1 j\n"), codes: BTreeMap::new(), glyphs: Vec::new() }
    }

    /// Paint a nonempty polyline with optional closure and a solid ink or paper fill.
    pub(super) fn path(&mut self, points: impl IntoIterator<Item = [f64; 2]>, width: f64, closed: bool, fill: Fill) {
        let _ = writeln!(self.ink, "{width:.5} w");
        if !path(&mut self.ink, points) {
            return;
        }
        if closed {
            self.ink.push_str("h\n");
        }
        self.ink.push_str(match fill {
            Fill::None => "S\n",
            Fill::Solid => "B\n",
            Fill::Background => "1 g B 0 g\n",
        });
    }

    /// Measure a single line with the same advances used in the emitted font.
    pub(super) fn text_width(&mut self, height: f64, text: &str) -> Result<f64, PdfError> {
        let mut width = 0.;
        for c in text.chars() {
            let i = self.code(c)?;
            width += self.glyphs[i].width;
        }
        Ok(width * height / 1000.)
    }

    /// Emit real text. The glyph codes only address the drawn face; a ToUnicode map independently
    /// preserves the original scalars for selection and search, including missing-glyph fallbacks.
    pub(super) fn text(&mut self, at: [f64; 2], height: f64, angle: f64, align: TextAlign, text: &str) -> Result<(), PdfError> {
        let (sin, cos) = angle.sin_cos();
        for (row, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let codes = line.chars().map(|c| self.code(c)).collect::<Result<Vec<_>, _>>()?;
            let width: f64 = codes.iter().map(|&i| self.glyphs[i].width * height / 1000.).sum();
            let x = match align {
                TextAlign::Start => 0.,
                TextAlign::Middle => -width / 2.,
                TextAlign::End => -width,
            };
            let y = -height * (0.5 + 1.5 * row as f64);
            let [tx, ty] = [at[0] + cos * x - sin * y, at[1] + sin * x + cos * y];
            let _ = writeln!(self.ink, "BT\n{cos:.6} {sin:.6} {:.6} {cos:.6} {tx:.5} {ty:.5} Tm", -sin);
            for run in codes.chunk_by(|a, b| a / CODES == b / CODES) {
                let _ = write!(self.ink, "/F{} {height:.5} Tf <", run[0] / CODES);
                for &i in run {
                    let _ = write!(self.ink, "{:02X}", i % CODES + 1);
                }
                self.ink.push_str("> Tj\n");
            }
            self.ink.push_str("ET\n");
        }
        Ok(())
    }

    /// Assign each Unicode scalar once, in stable first-use order.
    fn code(&mut self, c: char) -> Result<usize, PdfError> {
        if let Some(&code) = self.codes.get(&c) {
            return Ok(code);
        }
        let glyph = stroke_glyph(c).ok_or(PdfError::FontUnavailable)?;
        let scale = (1000. - GLYPH_STROKE) / glyph.cap_height;
        let width = (scale * glyph.advance + GLYPH_STROKE).round();
        let margin = GLYPH_STROKE / 2.;
        let bounds = glyph.paths.iter().flatten().fold([0_f64; 4], |[l, b, r, t], [x, y]| {
            [
                l.min((x * scale - margin).floor()),
                b.min((y * scale - margin).floor()),
                r.max((x * scale + margin).ceil()),
                t.max((y * scale + margin).ceil()),
            ]
        });
        let [l, b, r, t] = bounds;
        let mut program = format!("{width:.0} 0 {l:.0} {b:.0} {r:.0} {t:.0} d1\n{GLYPH_STROKE} w 1 J 1 j\n");
        for line in glyph.paths {
            if path(&mut program, line.iter().map(|[x, y]| [x * scale, y * scale])) {
                program.push_str("S\n");
            }
        }
        let code = self.glyphs.len();
        self.glyphs.push(Glyph { unicode: c, width, bounds, program });
        self.codes.insert(c, code);
        Ok(code)
    }

    /// Finish the page tree, font resources and classic byte-offset cross-reference table.
    pub(super) fn finish(mut self) -> Result<Vec<u8>, PdfError> {
        self.ink.push_str("Q\n");
        let mut objects = Objects(vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".into(),
            String::new(),
            stream(&self.ink),
        ]);
        let mut fonts = String::new();
        for (i, glyphs) in self.glyphs.chunks(CODES).enumerate() {
            let id = objects.font(glyphs);
            let _ = write!(fonts, "/F{i} {id} 0 R ");
        }
        let [width, height] = self.size;
        objects.0[2] =
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.5} {height:.5}] /Resources << /Font << {fonts}>> >> /Contents 4 0 R >>");
        objects.encode()
    }
}

/// Append PDF path operators and report whether a path was started.
fn path(out: &mut String, points: impl IntoIterator<Item = [f64; 2]>) -> bool {
    let mut points = points.into_iter();
    let Some([x, y]) = points.next() else { return false };
    let _ = writeln!(out, "{x:.5} {y:.5} m");
    for [x, y] in points {
        let _ = writeln!(out, "{x:.5} {y:.5} l");
    }
    true
}

struct Objects(Vec<String>);

impl Objects {
    /// Add an indirect object and return its one-based reference number.
    fn push(&mut self, object: String) -> usize {
        self.0.push(object);
        self.0.len()
    }

    /// Build the Type 3 CharProcs, widths and ToUnicode CMap for one code range.
    fn font(&mut self, glyphs: &[Glyph]) -> usize {
        let mut widths = String::new();
        let mut procs = String::new();
        let mut names = String::new();
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /SheetText def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
        );
        let notdef = self.push(stream("0 0 d0\n"));
        let _ = write!(procs, "/.notdef {notdef} 0 R ");
        for (chunk, group) in glyphs.chunks(100).enumerate() {
            let _ = writeln!(cmap, "{} beginbfchar", group.len());
            for (i, glyph) in group.iter().enumerate() {
                let code = chunk * 100 + i + 1;
                let id = self.push(stream(&glyph.program));
                let _ = write!(procs, "/g{code} {id} 0 R ");
                let _ = write!(names, "/g{code} ");
                let _ = write!(widths, "{:.0} ", glyph.width);
                let _ = write!(cmap, "<{code:02X}> <");
                for unit in glyph.unicode.encode_utf16(&mut [0; 2]) {
                    let _ = write!(cmap, "{unit:04X}");
                }
                cmap.push_str(">\n");
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let map = self.push(stream(&cmap));
        let [l, b, r, t] =
            glyphs.iter().fold([0_f64; 4], |[l, b, r, t], g| [l.min(g.bounds[0]), b.min(g.bounds[1]), r.max(g.bounds[2]), t.max(g.bounds[3])]);
        self.push(format!("<< /Type /Font /Subtype /Type3 /FontBBox [{l:.0} {b:.0} {r:.0} {t:.0}] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {procs}>> /Encoding << /Type /Encoding /Differences [1 {names}] >> /FirstChar 1 /LastChar {} /Widths [{widths}] /Resources << >> /ToUnicode {map} 0 R >>", glyphs.len()))
    }

    /// Serialize every offset in the fixed-width classic xref format without truncation.
    fn encode(self) -> Result<Vec<u8>, PdfError> {
        let mut out = String::from("%PDF-1.4\n");
        let count = self.0.len() + 1;
        let mut xref = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for (i, object) in self.0.into_iter().enumerate() {
            let _ = writeln!(xref, "{:010} 00000 n ", out.len());
            let _ = writeln!(out, "{} 0 obj\n{object}\nendobj", i + 1);
        }
        let start = out.len();
        if start as u64 >= 10_000_000_000 {
            return Err(PdfError::DocumentTooLarge);
        }
        let _ = write!(out, "{xref}trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n");
        Ok(out.into_bytes())
    }
}

/// Wrap an ASCII content stream; the final delimiter is not part of its declared bytes.
fn stream(content: &str) -> String {
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
}
