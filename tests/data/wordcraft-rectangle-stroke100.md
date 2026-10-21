# Native-generated Rectangle fixture

`wordcraft-rectangle-stroke100.docx` is a synthetic document generated for this repository, not third-party content. SHA256: `e70b198cbdcb030f3cede47396ec6c5907464eca85f4b3a48c3757b9f84c31e4` (6,774 bytes). No external assets, fonts or document text are embedded.

The original raw116 recipe is frozen as `/tmp/wordcraft-stroke-evidence/final-probe.rs`, SHA256 `c188ad0d0d9e3e1b2e79d74fa0982c2ffae9c3254831a6bbb6d9611e9059d665`. Its WordCraft DOCX writer is byte-identical between original upstream `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22` and the reviewed PDF-stroke fork. This is the original candidate `rectangle-100.docx` output, without manual ZIP/XML edits.

Reproduction recipe using public WordCraft model/writer calls: create Document::new; set last_section.page_w/page_h to440 and all four margins to20 points; create Paragraph::new; insert InlineObject::Shape at offset0 with ShapeKind::Rectangle, width160, height100, fillNone, strokeRgb(0,0,255), stroke_width100; Float uses BehindText, Page anchors for both axes, x/y140 and remaining defaults; story/freeform areNone, effects/extra default. Insert Block::Para at StoryRef::Body, Path::top(0), then wordcraft_docx::write. ZIP timestamps may differ across regeneration; model geometry and native rendering are the tested contract.

The native regression parses this same committed fixture, verifies geometry, writes/reopens it and exercises PNG/PDF output at72/144/300 DPI. The Python regression imports it through existing APIs; it does not require or advertise shape authoring. The95,95pt negative corner distinguishes the old mitered PDF from native round joins, while110,110pt is positive. These interior controls are separated from antialiased edges.
