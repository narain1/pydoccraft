# Python authoring API direction

The [pinned reference-family inventory](reference-api-inventory.md) now groups 194 representative members into 44 families and links existing criteria and evidence profiles. It remains a bounded discovery crosswalk, not complete SDK or PyMuPDF conformance.

Status: historical design direction. The implemented subset is documented in [the authoring guide](authoring.md); sketches below may include future operations. The [feature roadmap](../TODO.md) tracks implementation. Existing `doc_ex.open`, extraction, conversion, and PNG-returning rendering methods keep their contracts.

The overlap review consolidates the feature checklist into [14 canonical workstreams](roadmap-workstreams.md). Feature breadth is not API breadth: a feature may be delivered through existing primitives, ordinary properties and a short Python recipe. Shared naming does not require a universal document hierarchy, dispatcher or generic options dictionary.

## Reference projects and backend ownership

“OpenXML” is interpreted as Microsoft's [dotnet/Open-XML-SDK](https://github.com/dotnet/Open-XML-SDK). Its repository describes a low-level package/markup SDK. We reviewed its [WordprocessingDocument implementation](https://github.com/dotnet/Open-XML-SDK/blob/3139fdfd27414548a41555f7848d5728f6e71a42/src/DocumentFormat.OpenXml/Packaging/WordprocessingDocument.cs), [WordprocessingML schema inventory](https://github.com/dotnet/Open-XML-SDK/blob/3139fdfd27414548a41555f7848d5728f6e71a42/data/schemas/schemas_openxmlformats_org_wordprocessingml_2006_main.json), and the official references linked below. These provide feature discovery, not code to port or a dependency to install. This is a feature-family audit, not a completed member-by-member SDK conformance audit; API1 tracks that inventory.

Reference snapshot rechecked on 2026-10-20: Open XML SDK **v3.5.1**, immutable commit `3139fdfd27414548a41555f7848d5728f6e71a42`. The inspected WordprocessingML 2006 schema inventory contains 726 type entries and 101 enum entries; this count excludes separate DrawingML and Office-extension schema files. Package methods include `Create`, `Open` and `CreateFromTemplate`; representative schema members are listed below. These are feature references only, with no SDK code or runtime dependency added. The older Microsoft API links retain their explicit documentation view; that view is not the repository snapshot version. API1 remains open for the complete operation-by-operation evidence ledger.

| Referenced SDK members | Existing canonical ownership |
|---|---|
| `Paragraph`, `Run`, `Text`, `Break`, `TabChar`, `ParagraphProperties`, `RunProperties` | TXT1–TXT8; W04 |
| `Table`, `TableRow`, `TableCell`, `GridSpan`, `VerticalMerge`, `TableBorders`, `TableCellBorders`, `TableCellMargin`, `TableLook`, `TableCellSpacing` | TBL1–TBL12; W05 |
| `Style`, `BasedOn`, `NextParagraphStyle`, `LinkedStyle`, `DocDefaults` | STY1–STY6; W04 |
| `SectionProperties`, `PageSize`, `PageMargin`, `Columns`, `Column`, `PageBorders`, `DocumentBackground`, `HeaderReference`, `FooterReference` | LAY/BG features; W06 |
| `Drawing`, `Picture`, `TextBoxContent` | MED picture/drawing features; W06 |
| `Hyperlink`, `BookmarkStart`, `BookmarkEnd`, `SimpleField`, `FieldChar`, `FieldCode`, `FootnoteReference`, `EndnoteReference` | REF and LAY7; W06/W07/W08 |
| `Comment`, `CommentRangeStart`, `CommentRangeEnd`, `InsertedRun`, `DeletedRun`, `MoveFromRun`, `MoveToRun`, `RunPropertiesChange`, `ParagraphPropertiesChange` | REV1–REV6; W07 |
| `SdtBlock`, `SdtRun`, `SdtCell`, `SdtRow`, `SdtProperties`, `SdtContentBlock`, `DataBinding`, `Lock` | FORM3–FORM5; W08 |
| `Settings`, `DocumentProtection`, `Compatibility`, `HyphenationZone`, `DefaultTabStop`, `MirrorMargins` | LAY10 and PKG settings/protection; W03/W06 |

The table points to the existing workstream ownership ledger; it creates no new feature tickets or public classes. Class presence establishes schema representation, not layout, rendering, preservation or safe editing behavior in either SDK or ArtCraft.

The public API takes inspiration from [Pillow's Image API](https://pillow.readthedocs.io/en/stable/reference/Image.html): easy construction/opening, understandable objects, explicit operations, copying, and saving. Pillow itself is not required for document processing. Optional Pillow interoperation can consume our PNG bytes.

All document parsing, mutation, serialization, layout, rendering, and conversion must use the selected ArtCraft library crates: WordCraft for DOCX, PdfCraft for PDF, DeckCraft for PPTX. Python owns the ergonomic facade, validation, owned result values, Markdown, and explicit file I/O. No Open XML SDK, .NET runtime, Office automation, python-docx, python-pptx, PyMuPDF, or application executable is a fallback backend. Do not implement XML/PDF writers or a second layout engine in the adapter.

The Python API belongs to pydoccraft. It must remain stable if an upstream crate changes; format-specific adapters absorb differences. An absent public upstream capability becomes a tracked upstream requirement. It does not justify linking an application or silently reconstructing a lossy document.

## API rules

| Concern | Proposed contract |
|---|---|
| Entry points | `doc_ex.new("docx")` constructs an editable document; `doc_ex.open(source)` retains eager extraction; `source.edit()` makes an independent editable copy using original source data |
| Everyday vocabulary | `add_paragraph`, `add_table`, `add_image`, paragraph `add_run`, ordinary formatting properties, `copy`, and `save`; use existing rendering/conversion methods. Add conveniences only after repeated real use justifies them |
| Formats | A common lifecycle with format-specific capabilities; flow-oriented DOCX sections, fixed PDF pages, PPTX slides are distinct |
| Collections | Typed ordered collections; iteration, `len`, documented negative indexing; `table[row, column]` addresses a logical cell; no opaque tuple-based authoring objects |
| Values | Python strings, numbers, paths, bytes and typed values; readable string options such as `align="center"`; no required numeric flags or XML property bags |
| Units | Points for plain length numbers; `doc_ex.pt`, `mm`, `cm`, `inches` helpers; pixels only for raster results and DPI only for rasterization; percentages require an explicit typed value |
| Colors | Named colors, `#RRGGBB`, or explicit RGB/RGBA values with documented alpha support; theme references remain distinguishable from resolved colors |
| Mutations | Setters and edits modify only the editable document; `add_*` returns the created child; `copy()` returns an independent object; commands such as `save()` return `None` |
| Existing values | Current `Block`, `Table`, `Cell`, `Link`, and `Image` remain owned extraction values. Editable objects have separate types and ownership rules; extracted `Cell` is not an editable native cell |
| Persistence | `save(path_or_binary_stream)` writes the current native format; `to_bytes()` returns that format; `convert(target)` continues returning converted bytes; format/suffix conflicts are explicit errors |
| Lifecycle | Context managers release resources and never implicitly save; closing a source cannot invalidate its independent editable copy; edits invalidate layout caches and stale page handles predictably |
| Failures | Validate before mutation; error messages name the field/feature and constraint; unsupported operations fail before save rather than disappear from output |
| Rendering | Keep `render_page`/`get_pixmap` returning PNG bytes; saving or optional Pillow decoding are caller-side recipes. No new render alias or rendered-image wrapper |
| Capabilities | One development ledger and published support matrix by operation. Stable errors identify unsupported behavior; a public capability discovery framework is deferred |

Names below are draft acceptance examples. Final signatures must pass the pinned-upstream feasibility review and preserve the existing API. Basic calls should not require importing engine types, constructing relationship IDs, understanding twips/EMUs, or learning the Office XML schema.

## Draft DOCX workflow

```python
# HISTORICAL SKETCH — mixes implemented and proposed members.
# Use docs/cookbook.md for runnable examples with current signatures.
from pathlib import Path
import doc_ex

with doc_ex.new("docx") as report:
    section = report.sections[0]
    section.page_size = "A4"
    section.margins = doc_ex.mm(20)  # uniform margins
    report.background = "#FAFAF7"

    report.add_paragraph("Quarterly report", style="Heading 1")
    paragraph = report.add_paragraph("Prepared for ")
    paragraph.add_run("Acme", bold=True, color="#243C5A")

    table = report.add_table(
        [["Item", "Quantity", "Total"], ["Support", "2", "$200"]],
        header=True,
    )
    table.style = "Light Grid"
    table.columns[0].width = doc_ex.mm(90)
    table[0, 0].background = "#DFEAF5"
    table[1, 2].paragraphs[0].alignment = "right"

    report.add_image("logo.png", width=doc_ex.mm(30), alt="Acme logo")
    section.footer.add_paragraph("Confidential")
    report.save("report.docx")
    Path("report.pdf").write_bytes(report.convert("pdf"))
    Path("preview.png").write_bytes(report.render_page(0, dpi=300))
```

The sample's named table style must be supplied through the upstream writer or an attributed bundled style definition; it cannot rely on Word being installed. `add_table` does not automatically turn arbitrary Python objects into strings or invent numeric formatting. Provide strings first, with explicit formatters for data-oriented conveniences later.

## Draft editing workflow

```python
# PROPOSED — original input and extraction values remain unchanged.
import doc_ex

with doc_ex.open("template.docx") as source:
    with source.edit() as document:
        document.replace_text("{{customer}}", "Acme", scope="body")
        document.tables[0][1, 0].text = "Updated item"
        document.save("completed.docx")
```

Replacement must preserve surrounding runs, links, bookmarks, and comments; ambiguous replacements must report their policy. Assigning `cell.text` explicitly replaces that cell's content; it cannot masquerade as a format-preserving rich-text edit. Editing must use the original upstream model, not reconstruct the document from extracted plain text. A save that cannot preserve untouched content must fail with a diagnostic identifying the affected feature.

## Draft PDF and slide workflows

```python
# PROPOSED — separate format objects share lifecycle and rendering conventions.
import doc_ex

with doc_ex.new("pdf") as pdf:
    page = pdf.pages.add(size="A4")
    page.background = "#F4F6FA"
    page.draw_text("Invoice", at=(36, 48), font_size=18)
    pdf.save("invoice.pdf")

with doc_ex.new("pptx") as presentation:
    slide = presentation.slides.add(layout="blank")
    slide.background = "#13213C"
    slide.add_text("Quarterly results", box=(36, 36, 600, 100), color="white")
    presentation.save("slides.pptx")
```

Public positioned coordinates use top-left points; boxes are `(x, y, width, height)`. Existing extraction bounding boxes retain their documented contract. Fonts, text shaping, page drawing, and slide serialization must be upstream-backed. The pinned DeckCraft revision exposes the public PPTX writer used by the implemented authored profile. The [authoring guide](authoring.md) and [cookbook](cookbook.md) describe current methods; broader imported-document preservation remains open. See the [pinned-source upstream audit](upstream-roadmap.md).

## Small surface, composable workflows

- Reports use ordinary DOCX content followed by existing `convert("pdf")`; no report builder or second content schema.
- Named styles supply headings; no required `add_heading` synonym.
- Image backgrounds and image watermarks use `add_image` with supported anchor, layer, opacity and fit properties. Text watermarks reuse positioned text/shape primitives in a header or fixed page. No independent watermark engine or method family.
- Table input is plain rows. Callers format values with comprehensions or functions; no dataframe adapter or formatter registry in the core.
- Basic templates use literal text replacement and safe content copying. Repeated rows and multiple outputs use ordinary Python loops; no batch/template-language framework. Native content controls remain a separate real document capability.
- Native editing sessions, caching and validation-before-mutation remain private. Do not expose a transaction/cache/session framework. Ranges initially belong to a document revision and are explicitly invalidated by incompatible edits.
- Merge/split workflows use ordered content/page collections and copy/insert/delete operations. Do not add parallel combine/append/merge API families for the same primitive.
- Presets, selective privacy cleanup and Pillow interoperation start as documented recipes. Preserve limitations; do not promise a general sanitizer.

For example, **existing** PNG rendering already integrates with optional Pillow:

```python
from io import BytesIO
from PIL import Image  # optional caller dependency
import doc_ex

with doc_ex.open("report.docx") as document:
    with Image.open(BytesIO(document.render_page(0, dpi=300))) as preview:
        preview.thumbnail((800, 800))
        preview.save("thumbnail.png")
```

## Background semantics

[Open XML DocumentBackground](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.documentbackground?view=openxml-3.0.1) describes a document-wide background. Our API must distinguish that from paragraph shading, table-cell shading, and placed drawing objects.

| User operation | DOCX behavior to implement | PDF / PPTX behavior to implement |
|---|---|---|
| Page color | Document-wide native background; state whether each renderer/exporter honors it | Background on a selected fixed page / slide |
| Background image | Explicit image underlay with defined fit and anchor behavior | Image placed behind page / slide content |
| Watermark | Text/image object with section/header scope and layering rules | Explicit underlay or overlay on selected pages / slides |
| Cell/paragraph shading | Formatting attached to that semantic object | Table/paragraph formatting only where that semantic model exists |
| Preview background | Raster compositing option only; does not modify document content | Same distinction |

DOCX pages reflow after edits. Do not promise persistent “background on page 3” without a defined section/anchor strategy. Native background color can display differently across document viewers and exports; test saved DOCX, native PNG, and PDF separately. If upstream cannot reproduce a requested fill, return an unsupported-feature error instead of dropping it. New paths must not weaken existing conversion error behavior.

## Open XML feature crosswalk

These are reference concepts mapped to proposed Python concepts, not verified WordCraft capabilities. See the cited official pages for the underlying SDK concepts; roadmap completion requires a separate ArtCraft public-API audit.

| Reference family | Python-facing direction | Roadmap IDs | Reference |
|---|---|---|---|
| Package open/create/clone and document parts | `new`, `edit`, `copy`, `save`, owned document lifecycle | API, DOC, PKG | [SDK source](https://github.com/dotnet/Open-XML-SDK/blob/3139fdfd27414548a41555f7848d5728f6e71a42/src/DocumentFormat.OpenXml/Packaging/WordprocessingDocument.cs) |
| Body, paragraphs, runs, stories | Ordered blocks, paragraphs and runs; explicit auxiliary stories | TXT, LAY, REV | [Document structure](https://learn.microsoft.com/en-us/office/open-xml/word/structure-of-a-wordprocessingml-document) |
| Run properties and style definitions | Readable formatting properties and reusable named styles | TXT, STY | [Run properties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.runproperties?view=openxml-3.0.1), [styles](https://learn.microsoft.com/en-us/office/open-xml/word/how-to-create-and-add-a-paragraph-style-to-a-word-processing-document) |
| Numbering definitions and instances | Bullets, numbering levels and explicit restart/continue | TXT | [Numbering](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.numbering?view=openxml-3.0.1) |
| Tables, row/cell properties and grid | Editable tables, rows, columns, cells and merges | TBL | [Tables](https://learn.microsoft.com/en-us/office/open-xml/word/working-with-wordprocessingml-tables), [cell properties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tablecellproperties?view=openxml-3.0.1) |
| Background and drawing properties | Backgrounds, fills, watermarks and images | BG, MED | [Background](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.documentbackground?view=openxml-3.0.1), [pictures](https://learn.microsoft.com/en-us/office/open-xml/word/how-to-insert-a-picture-into-a-word-processing-document) |
| Section properties | Page setup, headers/footers and flow controls | LAY | [Sections](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.sectionproperties?view=openxml-3.0.1) |
| Field instructions and results | Typed fields with supported evaluation and stale-result reporting | REF | [Fields](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.fieldcode?view=openxml-3.0.1) |
| Structured document tags | Content controls and template bindings | FORM | [Content-control properties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.sdtproperties?view=openxml-3.0.1) |
| Revision elements | Tracked edits and explicit acceptance/rejection | REV | [Revision handling](https://learn.microsoft.com/en-us/office/open-xml/word/how-to-accept-all-revisions-in-a-word-processing-document) |
| Validation | Structured diagnostics with feature/location context | QA, PKG | [SDK validation reference](https://learn.microsoft.com/en-us/office/open-xml/word/how-to-validate-a-word-processing-document) |

## Completion and coverage

For each feature, record the reference concept, public Python operation, format, pinned upstream crate/symbol, and separate read/create/edit/preserve/render/export results. Use `unassessed`, `blocked upstream`, `partial` with a precise limitation, `verified` with evidence, or `not applicable`. An unchecked feature remains incomplete; it may contain implemented subsets or verified upstream blockers documented in the current authoring guide and validation evidence.

“Feature parity” targets document operations rather than every SDK class, overload, packaging primitive, or XML escape hatch. A broad inventory must still list unimplemented feature families; they cannot disappear from the matrix. Microsoft Word layout equivalence, complete Office field calculation, macro execution, signatures, and arbitrary embedded programs are not implied by a schema match.

Readiness needs an installed-wheel test that creates or edits, saves, reopens, extracts, renders and exports each advertised feature as applicable. Schema validity alone is insufficient. No .NET or Office component becomes a required test or runtime dependency. Independent viewer comparisons can be recorded as optional manual evidence, with redistributable baselines for normal CI.
