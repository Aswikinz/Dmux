//! The PDF summary (PLAN.md 4.7, 1, 2.4 and 5).
//!
//! Generated entirely in Rust with `printpdf`: no browser printing.  Liberation
//! Sans (regular and bold) is embedded from the bundled files under `fonts/`;
//! their permissive licence is `fonts/OFL-1.1.txt`.  The Dmux logo is embedded
//! from `assets/logo-128.rgba` (the app icon as raw RGBA pixels, so no image
//! decoder is needed).
//!
//! Page 1 reads top to bottom the way a reviewer signs: the overall verdict in a
//! banner (a word, never colour alone) with what it means, the coverage line and
//! counts, the check record (when the check ran, how long it took, when this
//! report was generated, which databases were compared), then everything that
//! needs attention.  Later sections hold every table, the per-table detail,
//! the connections, the integrity hashes, the limits of the method (the "what a
//! PASS does not prove" paragraph, the md5 threat model, the hash-integrity
//! sentence), the hook custom checks with script hashes and the sign-off block.
//! Every page carries the logo, the run ID and "Page N of M".
//!
//! The output is byte-deterministic for a given layout: the Info dictionary's
//! wall-clock dates and the trailer's random document ID are neutralised and all
//! resource IDs are fixed, so the golden/hash tests remain stable.  The only
//! time printed is the one the layout carries.

use std::path::Path;

use dmux_engine::results::RunResults;
use printpdf::{
    Color, LinePoint, Op, PaintMode, ParsedFont, PdfDocument, PdfFontHandle, PdfFontParseWarning,
    PdfSaveOptions, Point, Polygon, PolygonRing, Pt, RawImage, RawImageData, RawImageFormat, Rect,
    Rgb, TextItem, WindingOrder, XObjectId, XObjectTransform,
};

use crate::layout::{self, Layout, ReportOptions, TableSummary, Tone};
use crate::verify::HookEvidence;

const FONT_REGULAR: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../fonts/LiberationSans-Bold.ttf");
/// The app icon, 128 x 128 RGBA, 8 bits per channel.
const LOGO_RGBA: &[u8] = include_bytes!("../assets/logo-128.rgba");
const LOGO_PX: usize = 128;

/// A4 in points, with the text area inside the margins.
const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN_X: f32 = 48.0;
const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN_X;
/// Content starts below the running header and stops above the footer.
const CONTENT_TOP: f32 = 84.0;
const CONTENT_BOTTOM: f32 = PAGE_H - 58.0;

const BODY: f32 = 9.5;
const SMALL: f32 = 8.0;

/// An RGB colour, 0..=1 per channel.
#[derive(Clone, Copy)]
struct Rgb3(f32, f32, f32);

const fn hex(v: u32) -> Rgb3 {
    Rgb3(
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
    )
}

const INK: Rgb3 = hex(0x1d1d1f);
const MUTED: Rgb3 = hex(0x5f5f64);
const FAINT: Rgb3 = hex(0x8e8e93);
const RULE: Rgb3 = hex(0xdcdce2);
const ZEBRA: Rgb3 = hex(0xf6f6f9);
const HEAD_FILL: Rgb3 = hex(0xeeedfb);
const ACCENT: Rgb3 = hex(0x4a3df0);
const ACCENT_DARK: Rgb3 = hex(0x2a2487);

/// (strong, tint) colours for a tone.  The strong colour is dark enough for
/// text on the tint and on white (WCAG AA at body size).
fn tone_colours(t: Tone) -> (Rgb3, Rgb3) {
    match t {
        Tone::Good => (hex(0x0b6e4f), hex(0xe7f6ef)),
        Tone::Bad => (hex(0xb42318), hex(0xfdecea)),
        Tone::Caution => (hex(0x8a4b00), hex(0xfff3dc)),
        Tone::Neutral => (hex(0x3f3f46), hex(0xefeff2)),
    }
}

/// Errors rendering the PDF.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("cannot write PDF: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse the bundled font")]
    Font,
}

/// Write the PDF summary for verified results to `path`.
pub fn write_summary(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    results_sha256: &str,
    options: &ReportOptions,
    path: &Path,
) -> Result<(), PdfError> {
    let layout = layout::build(results, hooks, options);
    write_summary_layout(&layout, results_sha256, path)
}

/// Write a pre-built layout to disk.
pub fn write_summary_layout(
    layout: &Layout,
    results_sha256: &str,
    path: &Path,
) -> Result<(), PdfError> {
    let bytes = render_bytes(layout, results_sha256)?;
    crate::write_private(path, &bytes)?;
    Ok(())
}

/// Render a [`Layout`] to in-memory PDF bytes (the test entry point).
pub fn render_bytes(layout: &Layout, results_sha256: &str) -> Result<Vec<u8>, PdfError> {
    let doc = build_document(layout, results_sha256)?;
    Ok(neutralise_volatile(save_bytes(doc)?))
}

fn save_bytes(doc: PdfDocument) -> Result<Vec<u8>, PdfError> {
    let bytes = doc.save(&PdfSaveOptions::default(), &mut Vec::new());
    if bytes.is_empty() {
        return Err(PdfError::Font);
    }
    Ok(bytes)
}

/// A font registered in the document plus its metrics for measuring text.
struct Face {
    id: printpdf::FontId,
    metrics: ParsedFont,
}

impl Face {
    /// The advance width of `text` at `size`, in points.
    fn width(&self, text: &str, size: f32) -> f32 {
        let upem = f32::from(self.metrics.units_per_em.max(1));
        let units: f32 = text
            .chars()
            .map(|c| {
                self.metrics
                    .lookup_glyph_index(c as u32)
                    .and_then(|g| self.metrics.get_glyph_width(g))
                    .map_or(upem * 0.55, f32::from)
            })
            .sum();
        units / upem * size
    }

    fn handle(&self) -> PdfFontHandle {
        PdfFontHandle::External(self.id.clone())
    }
}

/// Build the printpdf document from a layout.
pub fn build_document(layout: &Layout, results_sha256: &str) -> Result<PdfDocument, PdfError> {
    let mut warnings_w = Vec::<PdfFontParseWarning>::new();
    let regular = ParsedFont::from_bytes(FONT_REGULAR, 0, &mut warnings_w).ok_or(PdfError::Font)?;
    let bold = ParsedFont::from_bytes(FONT_BOLD, 0, &mut warnings_w).ok_or(PdfError::Font)?;

    let mut doc = PdfDocument::new("Dmux migration verification report");
    // Register fonts and the logo under deterministic IDs (the `new()`
    // constructors are random, which would leak into every content stream and
    // defeat the byte-determinism the golden/hash tests rely on).
    let reg = Face {
        id: printpdf::FontId("FREGULAR".to_string()),
        metrics: regular.clone(),
    };
    let bld = Face {
        id: printpdf::FontId("FBOLD".to_string()),
        metrics: bold.clone(),
    };
    doc.resources
        .fonts
        .map
        .insert(reg.id.clone(), printpdf::PdfFont::new(regular));
    doc.resources
        .fonts
        .map
        .insert(bld.id.clone(), printpdf::PdfFont::new(bold));
    let logo = XObjectId("XLOGO".to_string());
    doc.resources.xobjects.map.insert(
        logo.clone(),
        printpdf::XObject::Image(RawImage {
            pixels: RawImageData::U8(LOGO_RGBA.to_vec()),
            width: LOGO_PX,
            height: LOGO_PX,
            data_format: RawImageFormat::RGBA8,
            tag: Vec::new(),
        }),
    );

    let mut c = Canvas::new(&reg, &bld, logo);
    compose(&mut c, layout, results_sha256);
    let pages = c.finish(&layout.run_id, layout.generated_at.as_deref());
    doc.with_pages(pages);
    Ok(doc)
}

// ---------------------------------------------------------------------------
// The canvas: a top-down cursor over A4 pages with a running header.
// Coordinates are points measured from the TOP-left; printpdf's origin is the
// bottom-left, so every op converts with `PAGE_H - y`.
// ---------------------------------------------------------------------------

struct Canvas<'a> {
    reg: &'a Face,
    bold: &'a Face,
    logo: XObjectId,
    pages: Vec<Vec<Op>>,
    ops: Vec<Op>,
    y: f32,
}

/// One table column: header, share of the width, right-aligned or not.
struct Col {
    head: &'static str,
    share: f32,
    right: bool,
}

const fn col(head: &'static str, share: f32, right: bool) -> Col {
    Col { head, share, right }
}

impl<'a> Canvas<'a> {
    fn new(reg: &'a Face, bold: &'a Face, logo: XObjectId) -> Self {
        let mut c = Canvas {
            reg,
            bold,
            logo,
            pages: Vec::new(),
            ops: Vec::new(),
            y: CONTENT_TOP,
        };
        c.header();
        c
    }

    fn face(&self, bold: bool) -> &'a Face {
        if bold {
            self.bold
        } else {
            self.reg
        }
    }

    // --- primitives --------------------------------------------------------

    fn fill(&mut self, c: Rgb3) {
        self.ops.push(Op::SetFillColor { col: colour(c) });
    }

    /// Draw `s` with its baseline at `baseline` (from the top).
    fn text(&mut self, x: f32, baseline: f32, s: &str, bold: bool, size: f32, c: Rgb3) {
        if s.is_empty() {
            return;
        }
        let font = self.face(bold).handle();
        self.ops.push(Op::StartTextSection);
        self.ops.push(Op::SetFont {
            font,
            size: Pt(size),
        });
        self.fill(c);
        self.ops.push(Op::SetTextCursor {
            pos: Point {
                x: Pt(x),
                y: Pt(PAGE_H - baseline),
            },
        });
        self.ops.push(Op::ShowText {
            items: vec![TextItem::Text(s.to_string())],
        });
        self.ops.push(Op::EndTextSection);
    }

    fn text_right(&mut self, right: f32, baseline: f32, s: &str, bold: bool, size: f32, c: Rgb3) {
        let w = self.face(bold).width(s, size);
        self.text(right - w, baseline, s, bold, size, c);
    }

    fn rect(&mut self, x: f32, top: f32, w: f32, h: f32, c: Rgb3) {
        self.fill(c);
        self.ops.push(Op::DrawRectangle {
            rectangle: Rect {
                x: Pt(x),
                y: Pt(PAGE_H - top - h),
                width: Pt(w),
                height: Pt(h),
                mode: Some(PaintMode::Fill),
                winding_order: None,
            },
        });
    }

    /// A filled rectangle with rounded corners of radius `r`.
    fn rounded(&mut self, x: f32, top: f32, w: f32, h: f32, r: f32, c: Rgb3) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let k = r * 0.552_284_8; // cubic approximation of a quarter circle
        let (l, rt) = (x, x + w);
        let (t, b) = (PAGE_H - top, PAGE_H - top - h);
        let p = |x: f32, y: f32, bezier: bool| LinePoint {
            p: Point { x: Pt(x), y: Pt(y) },
            bezier,
        };
        let points = vec![
            p(l + r, t, false),
            p(rt - r, t, false),
            p(rt - r + k, t, true),
            p(rt, t - r + k, true),
            p(rt, t - r, false),
            p(rt, b + r, false),
            p(rt, b + r - k, true),
            p(rt - r + k, b, true),
            p(rt - r, b, false),
            p(l + r, b, false),
            p(l + r - k, b, true),
            p(l, b + r - k, true),
            p(l, b + r, false),
            p(l, t - r, false),
            p(l, t - r + k, true),
            p(l + r - k, t, true),
            p(l + r, t, false),
        ];
        self.fill(c);
        self.ops.push(Op::DrawPolygon {
            polygon: Polygon {
                rings: vec![PolygonRing { points }],
                mode: PaintMode::Fill,
                winding_order: WindingOrder::NonZero,
            },
        });
    }

    fn hline(&mut self, x1: f32, x2: f32, y: f32, c: Rgb3, width: f32) {
        self.rect(x1, y - width / 2.0, x2 - x1, width, c);
    }

    fn logo_at(&mut self, x: f32, top: f32, size: f32) {
        // UseXobject scales an image to its pixel size at `dpi`; pick the dpi
        // that makes LOGO_PX pixels exactly `size` points wide.
        self.ops.push(Op::UseXobject {
            id: self.logo.clone(),
            transform: XObjectTransform {
                translate_x: Some(Pt(x)),
                translate_y: Some(Pt(PAGE_H - top - size)),
                dpi: Some(LOGO_PX as f32 * 72.0 / size),
                ..Default::default()
            },
        });
    }

    /// Word-wrap `s` to `max_w` points at `size`, breaking on spaces and, for
    /// a word longer than a line (a hash, a long table name), inside the word.
    fn wrap(&self, s: &str, max_w: f32, bold: bool, size: f32) -> Vec<String> {
        let face = self.face(bold);
        let mut out = Vec::new();
        for hard in s.split('\n') {
            let mut line = String::new();
            for word in hard.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if face.width(&candidate, size) <= max_w {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                // Hard-break a word wider than the line.
                let mut piece = String::new();
                for ch in word.chars() {
                    piece.push(ch);
                    if face.width(&piece, size) > max_w && piece.chars().count() > 1 {
                        piece.pop();
                        out.push(std::mem::take(&mut piece));
                        piece.push(ch);
                    }
                }
                line = piece;
            }
            out.push(line);
        }
        out
    }

    // --- pages ---------------------------------------------------------------

    /// The running header: logo, product name and report title, and a rule.
    fn header(&mut self) {
        self.logo_at(MARGIN_X, 26.0, 26.0);
        self.text(MARGIN_X + 34.0, 37.0, "Dmux", true, 12.0, INK);
        self.text(
            MARGIN_X + 34.0,
            50.0,
            "Migration verification report",
            false,
            SMALL,
            MUTED,
        );
        self.hline(MARGIN_X, PAGE_W - MARGIN_X, 62.0, RULE, 0.6);
        self.y = CONTENT_TOP;
    }

    fn new_page(&mut self) {
        self.pages.push(std::mem::take(&mut self.ops));
        self.header();
    }

    /// Start a new page unless `h` points still fit on this one.
    fn ensure(&mut self, h: f32) {
        if self.y + h > CONTENT_BOTTOM {
            self.new_page();
        }
    }

    /// Close the last page and stamp every footer ("Page N of M").
    fn finish(mut self, run_id: &str, generated: Option<&str>) -> Vec<printpdf::PdfPage> {
        self.pages.push(std::mem::take(&mut self.ops));
        let total = self.pages.len();
        let pages = std::mem::take(&mut self.pages);
        let mut out = Vec::with_capacity(total);
        for (i, ops) in pages.into_iter().enumerate() {
            self.ops = ops;
            let y = PAGE_H - 36.0;
            self.hline(MARGIN_X, PAGE_W - MARGIN_X, y - 12.0, RULE, 0.6);
            let left = match generated {
                Some(g) => format!("Run {run_id}  ·  Generated {g}"),
                None => format!("Run {run_id}"),
            };
            self.text(MARGIN_X, y, &left, false, 7.5, FAINT);
            let page = format!("Page {} of {total}", i + 1);
            self.text_right(PAGE_W - MARGIN_X, y, &page, false, 7.5, FAINT);
            out.push(printpdf::PdfPage::new(
                printpdf::Mm(PAGE_W * 0.352_778),
                printpdf::Mm(PAGE_H * 0.352_778),
                std::mem::take(&mut self.ops),
            ));
        }
        out
    }

    // --- blocks --------------------------------------------------------------

    /// A section heading with an accent bar.  Keeps at least `keep` points of
    /// what follows on the same page, so a heading is never orphaned.
    fn heading(&mut self, title: &str, keep: f32) {
        self.ensure(30.0 + keep);
        self.y += 10.0;
        self.rect(MARGIN_X, self.y, 3.0, 14.0, ACCENT);
        self.text(MARGIN_X + 10.0, self.y + 11.0, title, true, 12.5, INK);
        self.y += 24.0;
    }

    /// A wrapped paragraph at `x`, `w` wide.
    fn para_at(&mut self, x: f32, w: f32, s: &str, bold: bool, size: f32, c: Rgb3) {
        let leading = size * 1.45;
        for line in self.wrap(s, w, bold, size) {
            self.ensure(leading);
            self.text(x, self.y + size, &line, bold, size, c);
            self.y += leading;
        }
    }

    fn para(&mut self, s: &str) {
        self.para_at(MARGIN_X, CONTENT_W, s, false, BODY, INK);
        self.y += 4.0;
    }

    fn muted(&mut self, s: &str) {
        self.para_at(MARGIN_X, CONTENT_W, s, false, BODY, MUTED);
        self.y += 4.0;
    }

    fn bullets(&mut self, items: &[String]) {
        for item in items {
            let lines = self.wrap(item, CONTENT_W - 14.0, false, BODY);
            self.ensure(BODY * 1.45);
            self.rounded(MARGIN_X + 3.0, self.y + 4.0, 3.5, 3.5, 1.75, ACCENT);
            for line in lines {
                self.ensure(BODY * 1.45);
                self.text(MARGIN_X + 14.0, self.y + BODY, &line, false, BODY, INK);
                self.y += BODY * 1.45;
            }
            self.y += 2.0;
        }
        self.y += 2.0;
    }

    /// A two-column label / value list (the check record, integrity hashes).
    fn key_values(&mut self, rows: &[(&str, String)], label_w: f32, value_size: f32) {
        let value_w = CONTENT_W - label_w - 12.0;
        for (i, (label, value)) in rows.iter().enumerate() {
            let lines = self.wrap(value, value_w, false, value_size);
            let h = lines.len().max(1) as f32 * value_size * 1.45 + 9.0;
            self.ensure(h);
            if i % 2 == 0 {
                self.rect(MARGIN_X, self.y, CONTENT_W, h, ZEBRA);
            }
            self.text(
                MARGIN_X + 8.0,
                self.y + 6.0 + BODY,
                label,
                true,
                SMALL + 0.5,
                MUTED,
            );
            let mut ly = self.y + 6.0 + value_size;
            for line in lines {
                self.text(MARGIN_X + label_w, ly, &line, false, value_size, INK);
                ly += value_size * 1.45;
            }
            self.y += h;
        }
        self.y += 8.0;
    }

    /// A data table with a shaded header row (repeated on every page it spans),
    /// wrapped cells and light zebra rows.  `verdict_col` is the column whose
    /// cells are verdict words, drawn bold in their tone colour.
    fn table(&mut self, cols: &[Col], rows: &[Vec<String>], verdict_col: Option<usize>) {
        let total: f32 = cols.iter().map(|c| c.share).sum();
        let widths: Vec<f32> = cols.iter().map(|c| c.share / total * CONTENT_W).collect();
        let pad = 5.0;
        let size = SMALL + 0.5;
        let leading = size * 1.4;
        let header_h = 18.0;

        let draw_header = |c: &mut Canvas| {
            c.rect(MARGIN_X, c.y, CONTENT_W, header_h, HEAD_FILL);
            let mut x = MARGIN_X;
            for (col, w) in cols.iter().zip(&widths) {
                if col.right {
                    c.text_right(x + w - pad, c.y + 12.0, col.head, true, SMALL, ACCENT_DARK);
                } else {
                    c.text(x + pad, c.y + 12.0, col.head, true, SMALL, ACCENT_DARK);
                }
                x += w;
            }
            c.y += header_h;
        };

        // Keep the header with at least one row.
        self.ensure(header_h + leading + 10.0);
        draw_header(self);
        for (ri, row) in rows.iter().enumerate() {
            let cells: Vec<Vec<String>> = row
                .iter()
                .zip(&widths)
                .enumerate()
                .map(|(ci, (cell, w))| {
                    let bold = verdict_col == Some(ci);
                    self.wrap(cell, w - 2.0 * pad, bold, size)
                })
                .collect();
            let lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let h = lines as f32 * leading + 8.0;
            if self.y + h > CONTENT_BOTTOM {
                self.new_page();
                draw_header(self);
            }
            if ri % 2 == 1 {
                self.rect(MARGIN_X, self.y, CONTENT_W, h, ZEBRA);
            }
            let mut x = MARGIN_X;
            for (ci, ((cell_lines, col), w)) in cells.iter().zip(cols).zip(&widths).enumerate() {
                let is_verdict = verdict_col == Some(ci);
                let colour = if is_verdict {
                    tone_colours(layout::verdict_tone(&row[ci])).0
                } else {
                    INK
                };
                let mut ly = self.y + 4.0 + size;
                for line in cell_lines {
                    if col.right {
                        self.text_right(x + w - pad, ly, line, is_verdict, size, colour);
                    } else {
                        self.text(x + pad, ly, line, is_verdict, size, colour);
                    }
                    ly += leading;
                }
                x += w;
            }
            self.y += h;
            self.hline(MARGIN_X, PAGE_W - MARGIN_X, self.y, RULE, 0.4);
        }
        self.y += 10.0;
    }

    /// A small rounded label in a tone, e.g. a table's verdict.  Returns its width.
    fn pill(&mut self, x: f32, top: f32, word: &str) -> f32 {
        let (strong, tint) = tone_colours(layout::verdict_tone(word));
        let w = self.bold.width(word, 7.5) + 12.0;
        self.rounded(x, top, w, 13.0, 6.5, tint);
        self.text(x + 6.0, top + 9.3, word, true, 7.5, strong);
        w
    }
}

fn colour(c: Rgb3) -> Color {
    Color::Rgb(Rgb {
        r: c.0,
        g: c.1,
        b: c.2,
        icc_profile: None,
    })
}

// ---------------------------------------------------------------------------
// The report content, in reading order.
// ---------------------------------------------------------------------------

fn compose(c: &mut Canvas, layout: &Layout, results_sha256: &str) {
    title_block(c, layout);
    verdict_banner(c, layout);
    stat_tiles(c, layout);

    c.heading("Check record", 60.0);
    c.key_values(&layout::record_rows(layout), 120.0, BODY);

    needs_attention(c, layout);
    page_one_lists(c, layout);
    all_tables(c, layout);
    table_details(c, layout);
    connections(c, layout);
    custom_checks(c, layout);
    integrity(c, layout, results_sha256);
    limits(c);
    sign_off(c);
}

fn title_block(c: &mut Canvas, layout: &Layout) {
    c.text(
        MARGIN_X,
        c.y + 20.0,
        "Migration verification report",
        true,
        21.0,
        INK,
    );
    c.y += 30.0;
    let sub = match (&layout.old_source, &layout.new_source) {
        (Some(o), Some(n)) => format!("{o}  →  {n}"),
        _ => format!("Run {}", layout.run_id),
    };
    c.para_at(MARGIN_X, CONTENT_W, &sub, false, BODY, MUTED);
    c.y += 8.0;
}

fn verdict_banner(c: &mut Canvas, layout: &Layout) {
    let shown = layout::verdict_display(
        &layout.verdict_word,
        layout.options_applied,
        layout.options_recorded,
    );
    let (strong, tint) = tone_colours(layout::verdict_tone(&layout.verdict_word));
    let inner_w = CONTENT_W - 40.0;
    let word_size = if shown.chars().count() > 22 {
        17.0
    } else {
        26.0
    };
    let word_lines = c.wrap(&shown, inner_w, true, word_size);
    let meaning = c.wrap(
        layout::verdict_meaning(&layout.verdict_word),
        inner_w,
        false,
        BODY + 0.5,
    );
    let coverage = c.wrap(&layout.coverage_line, inner_w, true, BODY);
    let h = 26.0
        + word_lines.len() as f32 * word_size * 1.15
        + 6.0
        + meaning.len() as f32 * (BODY + 0.5) * 1.45
        + 16.0
        + coverage.len() as f32 * BODY * 1.45
        + 14.0;
    c.ensure(h);
    let top = c.y;
    c.rounded(MARGIN_X, top, CONTENT_W, h, 8.0, tint);
    c.rounded(MARGIN_X, top, 6.0, h, 3.0, strong);
    c.rect(MARGIN_X + 3.0, top, 3.0, h, strong);
    let x = MARGIN_X + 24.0;
    let mut y = top + 20.0;
    c.text(x, y, "OVERALL VERDICT", true, 7.5, strong);
    y += 6.0;
    for line in &word_lines {
        y += word_size * 1.15;
        c.text(x, y - word_size * 0.2, line, true, word_size, strong);
    }
    y += 6.0;
    for line in &meaning {
        y += (BODY + 0.5) * 1.45;
        c.text(x, y - 3.0, line, false, BODY + 0.5, INK);
    }
    y += 16.0;
    c.text(x, y + 2.0, "Coverage", false, SMALL, MUTED);
    for line in &coverage {
        y += BODY * 1.45;
        c.text(x, y + 2.0, line, true, BODY, INK);
    }
    c.y = top + h + 12.0;
}

fn stat_tiles(c: &mut Canvas, layout: &Layout) {
    let attention = layout
        .tables
        .iter()
        .filter(|t| t.verdict_word != "PASS")
        .count();
    let tiles = [
        ("In scope", layout.in_scope.to_string()),
        ("Fully checked", layout.fully_checked.to_string()),
        ("Counts only", layout.counts_only.to_string()),
        ("Structure only", layout.structure_only.to_string()),
        ("Ignored", layout.ignored.to_string()),
        ("Tables not passing", attention.to_string()),
    ];
    let gap = 6.0;
    let w = (CONTENT_W - gap * (tiles.len() as f32 - 1.0)) / tiles.len() as f32;
    let h = 46.0;
    c.ensure(h + 6.0);
    let top = c.y;
    for (i, (label, value)) in tiles.iter().enumerate() {
        let x = MARGIN_X + i as f32 * (w + gap);
        let flagged = i == tiles.len() - 1 && attention > 0;
        let (fg, bg) = if flagged {
            tone_colours(Tone::Bad)
        } else {
            (INK, ZEBRA)
        };
        c.rounded(x, top, w, h, 6.0, bg);
        c.text(x + 9.0, top + 22.0, value, true, 16.0, fg);
        c.text(x + 9.0, top + 36.0, label, false, 7.5, MUTED);
    }
    c.y = top + h + 6.0;
}

fn needs_attention(c: &mut Canvas, layout: &Layout) {
    let failing: Vec<&TableSummary> = layout
        .tables
        .iter()
        .filter(|t| t.verdict_word != "PASS")
        .collect();
    c.heading("Failures and findings", 40.0);
    if failing.is_empty() {
        c.muted("None. Every table that was checked passed at its level.");
        return;
    }
    for t in failing {
        let reasons: Vec<String> = if t.reasons.is_empty() {
            vec!["no reason recorded".to_string()]
        } else {
            t.reasons.clone()
        };
        let name_lines = c.wrap(&t.table, CONTENT_W - 150.0, true, 10.0);
        c.ensure(24.0 + reasons.len() as f32 * BODY * 1.45);
        let top = c.y;
        let pw = c.pill(MARGIN_X, top, &t.verdict_word);
        let mut y = top;
        for line in &name_lines {
            c.text(MARGIN_X + pw + 8.0, y + 10.0, line, true, 10.0, INK);
            y += 13.0;
        }
        c.text_right(PAGE_W - MARGIN_X, top + 10.0, &t.level, false, SMALL, MUTED);
        c.y = y + 4.0;
        c.bullets(&reasons);
        c.y += 6.0;
    }
}

fn page_one_lists(c: &mut Canvas, layout: &Layout) {
    if !layout.options_recorded {
        c.heading("Options applied", 30.0);
        c.para(layout::OPTIONS_NOT_RECORDED);
    } else if !layout.options_applied_lines.is_empty() {
        c.heading("Options applied", 30.0);
        c.muted(
            "These column options loosened the comparison. Each is part of what was signed off.",
        );
        c.bullets(&layout.options_applied_lines);
    }

    c.heading("Ignored items", 30.0);
    if layout.ignored_items.is_empty() {
        c.muted("None. Nothing in scope was skipped.");
    } else {
        let rows: Vec<Vec<String>> = layout
            .ignored_items
            .iter()
            .map(|i| vec![i.path.clone(), i.reason.clone()])
            .collect();
        c.table(
            &[col("Item", 2.0, false), col("Reason", 3.0, false)],
            &rows,
            None,
        );
    }

    c.heading("Structure findings", 30.0);
    if layout.structure_findings.is_empty() {
        c.muted("None. The structure matched after the mapping was applied.");
    } else {
        let rows: Vec<Vec<String>> = layout
            .structure_findings
            .iter()
            .map(|f| vec![f.object.clone(), f.verdict.clone(), f.message.clone()])
            .collect();
        c.table(
            &[
                col("Object", 2.2, false),
                col("Verdict", 1.1, false),
                col("Finding", 3.0, false),
            ],
            &rows,
            Some(1),
        );
    }

    let mut warnings = layout.warnings.clone();
    for t in &layout.changed_during_check {
        warnings.push(format!("{t} changed during the check"));
    }
    if !warnings.is_empty() {
        c.heading("Warnings", 30.0);
        c.bullets(&warnings);
    }

    if !layout.suggestions.is_empty() {
        c.heading("Suggested matches", 30.0);
        let rows: Vec<Vec<String>> = layout
            .suggestions
            .iter()
            .map(|s| vec![s.path.clone(), s.candidate.clone(), s.confidence.clone()])
            .collect();
        c.table(
            &[
                col("Unmapped", 2.0, false),
                col("Possible match", 2.0, false),
                col("Confidence", 1.0, true),
            ],
            &rows,
            None,
        );
    }
}

fn all_tables(c: &mut Canvas, layout: &Layout) {
    c.heading("Per-table results", 50.0);
    if layout.tables.is_empty() {
        c.muted("No tables were compared.");
        return;
    }
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
    let rows: Vec<Vec<String>> = layout
        .tables
        .iter()
        .map(|t| {
            vec![
                t.table.clone(),
                t.level.clone(),
                t.verdict_word.clone(),
                dash(&t.old_rows),
                dash(&t.new_rows),
                t.missing_rows.clone(),
                t.extra_rows.clone(),
                t.changed_rows.clone(),
            ]
        })
        .collect();
    c.table(
        &[
            col("Table", 2.8, false),
            col("Level", 1.3, false),
            col("Verdict", 1.9, false),
            col("Old rows", 1.1, true),
            col("New rows", 1.1, true),
            col("Missing", 0.95, true),
            col("Extra", 0.85, true),
            col("Changed", 0.95, true),
        ],
        &rows,
        Some(2),
    );
}

/// Detail for every table that has more to say than its row in the table:
/// reasons, bucket counts, changed columns, notes, control totals, keys.
fn table_details(c: &mut Canvas, layout: &Layout) {
    let detailed: Vec<&TableSummary> = layout
        .tables
        .iter()
        .filter(|t| {
            t.verdict_word != "PASS"
                || !t.changed_columns.is_empty()
                || !t.plain_words.is_empty()
                || !t.column_notes.is_empty()
                || !t.control_totals.is_empty()
                || !t.keyed_rows.is_empty()
                || !t.keys.is_empty()
        })
        .collect();
    if detailed.is_empty() {
        return;
    }
    c.heading("Table details", 60.0);
    for t in detailed {
        let key_label = if t.keyed_rows.len() == 1 {
            "Row key".to_string()
        } else {
            format!("Row keys ({})", t.keyed_rows.len())
        };
        let mut rows: Vec<(&str, String)> = vec![("Level", t.level.clone())];
        rows.push((
            "Buckets",
            format!(
                "{} differing, {} missing, {} extra",
                t.differing_buckets, t.missing_buckets, t.extra_buckets
            ),
        ));
        rows.push((
            "Rows",
            format!(
                "{} missing, {} extra, {} changed",
                t.missing_rows, t.extra_rows, t.changed_rows
            ),
        ));
        if !t.reasons.is_empty() {
            rows.push(("Reasons", t.reasons.join("; ")));
        }
        if !t.changed_columns.is_empty() {
            rows.push(("Changed columns", t.changed_columns.join(", ")));
        }
        for w in &t.plain_words {
            rows.push(("Note", w.clone()));
        }
        for n in &t.column_notes {
            rows.push(("Column note", n.clone()));
        }
        for ct in &t.control_totals {
            rows.push(("Control total", ct.clone()));
        }
        if !t.keys.is_empty() {
            rows.push(("Keys / covered columns", t.keys.join(", ")));
        }
        if !t.keyed_rows.is_empty() {
            rows.push((&key_label, t.keyed_rows.join(";  ")));
        }
        c.ensure(40.0);
        let top = c.y;
        let pw = c.pill(MARGIN_X, top, &t.verdict_word);
        let name_lines = c.wrap(&t.table, CONTENT_W - pw - 10.0, true, 10.5);
        let mut y = top;
        for line in &name_lines {
            c.text(MARGIN_X + pw + 8.0, y + 10.0, line, true, 10.5, INK);
            y += 14.0;
        }
        c.y = y + 4.0;
        c.key_values(&rows, 120.0, SMALL + 0.5);
    }
}

fn connections(c: &mut Canvas, layout: &Layout) {
    if layout.connections.is_empty() {
        return;
    }
    c.heading("Connections", 40.0);
    c.bullets(&layout.connections);
}

fn custom_checks(c: &mut Canvas, layout: &Layout) {
    if layout.custom_checks.is_empty() && layout.hook_scripts.is_empty() {
        return;
    }
    c.heading("Custom checks (Python hooks)", 40.0);
    c.muted("These checks are scripts your team supplied. Their results count toward the overall verdict.");
    if !layout.custom_checks.is_empty() {
        let rows: Vec<Vec<String>> = layout
            .custom_checks
            .iter()
            .map(|k| {
                vec![
                    k.id.clone(),
                    k.severity.to_uppercase(),
                    k.table.clone().unwrap_or_else(|| "-".to_string()),
                    k.message.clone(),
                ]
            })
            .collect();
        c.table(
            &[
                col("Check", 1.2, false),
                col("Result", 0.9, false),
                col("Table", 1.8, false),
                col("Message", 3.0, false),
            ],
            &rows,
            None,
        );
    }
    let warnings: Vec<String> = layout
        .hook_warnings
        .iter()
        .map(|w| format!("hook warning: {w}"))
        .collect();
    c.bullets(&warnings);
    if !layout.hook_scripts.is_empty() {
        let rows: Vec<(&str, String)> = layout
            .hook_scripts
            .iter()
            .map(|s| ("Script", format!("{}  SHA-256 {}", s.script, s.sha256)))
            .collect();
        c.key_values(&rows, 70.0, SMALL);
    }
}

fn integrity(c: &mut Canvas, layout: &Layout, results_sha256: &str) {
    c.heading("Run identity and integrity", 80.0);
    let mut rows: Vec<(&str, String)> = vec![
        ("Run ID", layout.run_id.clone()),
        ("Tool version", layout.tool_version.clone()),
        ("Results SHA-256", results_sha256.to_string()),
        ("Mapping hash", layout.mapping_hash.clone()),
        ("Results schema version", layout.results_version.clone()),
    ];
    if !layout.definition_hash.is_empty() {
        rows.push(("Definition hash", layout.definition_hash.clone()));
    }
    if let Some(v) = &layout.capture_sha256_old {
        rows.push(("Old capture SHA-256", v.clone()));
    }
    if let Some(v) = &layout.capture_sha256_new {
        rows.push(("New capture SHA-256", v.clone()));
    }
    if let Some(v) = &layout.wal_start_lsn {
        rows.push(("Write-ahead log start", v.clone()));
    }
    if let Some(v) = &layout.wal_end_lsn {
        rows.push(("Write-ahead log end", v.clone()));
    }
    c.key_values(&rows, 130.0, SMALL + 0.5);
}

fn limits(c: &mut Canvas) {
    c.heading("What a PASS does not prove", 40.0);
    c.para(layout::WHAT_PASS_DOES_NOT_PROVE);
    c.heading("Limit of the row hashes", 40.0);
    c.para(layout::MD5_THREAT_MODEL);
    c.heading("Trust and audit", 40.0);
    c.para(layout::TAMPER_EVIDENT_NOT_PROOF);
}

fn sign_off(c: &mut Canvas) {
    let block = layout::sign_off_block();
    let (title, statement, fields) = (block[0], block[1], &block[2..]);
    c.heading(title, 150.0);
    c.para(statement);
    c.y += 6.0;
    // Two columns of signature fields: a label above a line to write on.
    let gap = 24.0;
    let w = (CONTENT_W - gap) / 2.0;
    for pair in fields.chunks(2) {
        c.ensure(40.0);
        for (i, field) in pair.iter().enumerate() {
            let label = field.split(':').next().unwrap_or(field).trim();
            let x = MARGIN_X + i as f32 * (w + gap);
            c.text(x, c.y + 10.0, label, true, SMALL, MUTED);
            c.hline(x, x + w, c.y + 34.0, MUTED, 0.7);
        }
        c.y += 46.0;
    }
}

/// Make the rendered bytes deterministic by overwriting the wall-clock values
/// printpdf writes (the Info dictionary's creation/modification dates and the
/// trailer's random document ID) with fixed same-length values.
fn neutralise_volatile(mut pdf: Vec<u8>) -> Vec<u8> {
    let fixed_date: &[u8] = b"D:19700101000000+00'00'";
    for key in [b"/CreationDate(".as_slice(), b"/ModDate(".as_slice()] {
        overwrite_value(&mut pdf, key, fixed_date);
    }
    // The /ID holds two random 32-char literal strings; overwrite with zeros.
    let mut id_pos = 0;
    while let Some(id) = find_bytes(&pdf, b"/ID", id_pos) {
        id_pos = id + b"/ID".len();
        let mut pos = id;
        for _ in 0..2 {
            let Some(open) = find_bytes(&pdf, b"(", pos) else {
                break;
            };
            let Some(close) = find_bytes(&pdf, b")", open + 1) else {
                break;
            };
            for b in pdf.iter_mut().take(close).skip(open + 1) {
                *b = b'0';
            }
            pos = close + 1;
        }
    }
    pdf
}

fn overwrite_value(pdf: &mut [u8], key: &[u8], fixed: &[u8]) {
    let mut start = 0;
    while let Some(p) = find_bytes(pdf, key, start) {
        let value_start = p + key.len();
        let Some(close) = find_bytes(pdf, b")", value_start) else {
            break;
        };
        for (i, b) in pdf.iter_mut().take(close).skip(value_start).enumerate() {
            *b = if i < fixed.len() { fixed[i] } else { b'0' };
        }
        start = close + 1;
    }
}

fn find_bytes(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::build;
    use dmux_engine::results::{
        finalise, ChangedRowExport, CheckLevel, RunResults, TableDetail, TableResult, Verdict,
        RESULTS_VERSION,
    };
    use printpdf::PdfParseOptions;
    use std::collections::BTreeMap;

    const SENTINEL: &str = "SECRET-ROW-VALUE-999";

    fn fixture() -> RunResults {
        let mut tables = BTreeMap::new();
        tables.insert(
            "demo_bank.accounts".into(),
            TableResult {
                table: "demo_bank.accounts".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: Some(100),
                new_rows: Some(99),
                columns_covered: Some(vec!["id".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: Some(1),
                extra_rows: Some(0),
                changed_rows: Some(0),
                reasons: vec!["row counts differ".into()],
                column_notes: vec![],
            },
        );
        let details = BTreeMap::from([(
            "demo_bank.accounts".into(),
            TableDetail {
                changed_columns: vec!["balance".into()],
                changed_rows: vec![ChangedRowExport {
                    key: Some(vec![Some(SENTINEL.into())]),
                    columns: vec!["balance".into()],
                }],
                missing_keys: None,
                extra_keys: None,
                control_totals: vec![],
                plain_words: vec!["a balance changed".into()],
            },
        )]);
        let r = RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh".into(),
            verdict: Verdict::Fail,
            coverage_line:
                "0 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            tables,
            details,
            ..RunResults::default()
        };
        finalise(r)
    }

    /// Read all the text back out of a rendered PDF using printpdf's own parser,
    /// so the assertions match exactly what a real reader (and pdftotext) would
    /// recover from the ToUnicode CMaps.
    fn extract_texts(pdf: &[u8]) -> Vec<String> {
        let mut warnings = Vec::new();
        let doc = PdfDocument::parse(pdf, &PdfParseOptions::default(), &mut warnings)
            .expect("rendered PDF must parse");
        doc.extract_text()
            .into_iter()
            .map(|page| page.join("\n"))
            .collect()
    }

    fn any_contains(texts: &[String], needle: &str) -> bool {
        texts.iter().any(|t| t.contains(needle))
    }

    #[test]
    fn pdf_omits_drilldown_keys_by_default_and_includes_them_when_ticked() {
        let r = fixture();
        let without = build(&r, None, &ReportOptions::default());
        let with = build(
            &r,
            None,
            &ReportOptions {
                include_keys: true,
                ..Default::default()
            },
        );

        let t_without = extract_texts(&render_bytes(&without, "abc").unwrap());
        let t_with = extract_texts(&render_bytes(&with, "abc").unwrap());

        assert!(
            !any_contains(&t_without, SENTINEL),
            "drill-down key must be absent by default ({t_without:?})"
        );
        assert!(
            any_contains(&t_with, SENTINEL),
            "drill-down key present when ticked ({t_with:?})"
        );
    }

    #[test]
    fn pdf_never_prints_a_row_value_sentinel() {
        let r = fixture();
        let layout = build(&r, None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(
            !any_contains(&texts, SENTINEL),
            "sentinel leaked into {texts:?}"
        );
    }

    #[test]
    fn pdf_strips_bidi_controls_from_rendered_text() {
        let mut r = fixture();
        r.tables.get_mut("demo_bank.accounts").unwrap().reasons =
            vec!["\u{202E}benign".to_string()];
        let r = finalise(r);
        let layout = build(&r, None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        let joined = texts.concat();
        assert!(
            !joined.contains('\u{202E}'),
            "bidi control leaked: {joined:?}"
        );
        assert!(
            joined.contains("benign"),
            "reason still present after strip"
        );
    }

    #[test]
    fn pdf_carries_mandatory_prose_and_signoff() {
        let layout = build(&fixture(), None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(
            any_contains(&texts, "What a PASS does not prove"),
            "{texts:?}"
        );
        assert!(any_contains(&texts, "md5"));
        assert!(any_contains(&texts, "These hashes detect corruption"));
        assert!(any_contains(&texts, "Sign-off"));
        assert!(any_contains(&texts, "Signature"));
        assert!(any_contains(&texts, "Coverage"));
        assert!(any_contains(&texts, "FAIL"));
    }

    #[test]
    fn pdf_renders_hook_custom_checks_and_script_hashes() {
        use crate::verify::{HookEvidence, HookFinding, HookScript};
        let hooks = HookEvidence {
            custom_checks: vec![HookFinding {
                id: "c1".into(),
                severity: "fail".into(),
                message: "a custom check failed".into(),
                table: Some("demo_bank.accounts".into()),
            }],
            scripts: vec![HookScript {
                script: "hook.py".into(),
                sha256: "ab".repeat(32),
            }],
            warnings: vec!["hook warning".into()],
        };
        let layout = build(&fixture(), Some(&hooks), &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(
            any_contains(&texts, "Custom checks (Python hooks)"),
            "{texts:?}"
        );
        assert!(any_contains(&texts, "a custom check failed"));
        assert!(any_contains(&texts, "hook.py"), "{texts:?}");
        let joined = texts.concat();
        assert!(joined.contains(&"ab".repeat(32)), "script hash present");
    }

    #[test]
    fn pdf_carries_the_check_record_logo_and_page_numbers() {
        let mut r = fixture();
        r.started_at = Some("2026-11-18T09:00:00Z".into());
        r.finished_at = Some("2026-11-18T09:02:05Z".into());
        r.old_source = Some("bank on old-db:5432 (login dmux)".into());
        r.new_source = Some("bank on new-db:5432 (login dmux)".into());
        let r = finalise(r);
        let opts = ReportOptions {
            generated_at: Some("2026-11-18T10:00:00Z".into()),
            ..Default::default()
        };
        let bytes = render_bytes(&build(&r, None, &opts), "abc").unwrap();
        let texts = extract_texts(&bytes);
        for needle in [
            "Check record",
            "18 Nov 2026, 09:00:00 UTC",
            "18 Nov 2026, 09:02:05 UTC",
            "2 min 5 s",
            "18 Nov 2026, 10:00:00 UTC",
            "bank on old-db:5432 (login dmux)",
            "Page 1 of",
            "OVERALL VERDICT",
        ] {
            assert!(any_contains(&texts, needle), "{needle} missing: {texts:?}");
        }
        assert!(
            find_bytes(&bytes, b"XLOGO", 0).is_some(),
            "logo image embedded"
        );
    }

    #[test]
    fn an_older_results_file_says_the_record_was_not_kept() {
        let layout = build(&fixture(), None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(any_contains(&texts, "not recorded (older results file)"));
        // No generated time was given, so none is printed.
        assert!(!any_contains(&texts, "Report generated"), "{texts:?}");
    }

    #[test]
    fn long_text_wraps_inside_the_page() {
        let mut r = fixture();
        r.tables.get_mut("demo_bank.accounts").unwrap().reasons = vec!["word ".repeat(400)];
        let r = finalise(r);
        let layout = build(&r, None, &ReportOptions::default());
        let doc = build_document(&layout, "abc").unwrap();
        assert!(doc.pages.len() >= 2, "long content paginates");
    }

    #[test]
    fn pdf_output_is_deterministic() {
        let layout = build(&fixture(), None, &ReportOptions::default());
        let a = render_bytes(&layout, "abc").unwrap();
        let b = render_bytes(&layout, "abc").unwrap();
        assert_eq!(a, b, "PDF must be byte-deterministic");
    }

    #[test]
    fn pdf_shows_pass_with_options_and_the_options_list() {
        use dmux_engine::results::ColumnNote;
        let mut r = fixture();
        r.verdict = Verdict::Pass;
        for t in r.tables.values_mut() {
            t.verdict = Verdict::Pass;
            t.reasons.clear();
        }
        r.tables.get_mut("demo_bank.accounts").unwrap().column_notes = vec![ColumnNote {
            column: "balance".into(),
            kind: "option".into(),
            note: "round to 2 decimals".into(),
        }];
        r.options_applied = true;
        let r = finalise(r);
        let layout = build(&r, None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(any_contains(&texts, "PASS (with options)"), "{texts:?}");
        assert!(any_contains(&texts, "Options applied"), "{texts:?}");
        assert!(any_contains(&texts, "round to 2 decimals"), "{texts:?}");
    }
}
