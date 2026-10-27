# Table acceptance corpus

The six generated fixtures in [test_table_corpus.py](../tests/test_table_corpus.py) exercise combinations through the public API. They require no downloaded documents, fonts or viewer programs. `build_case(document, name)` builds a named fixture for inspection; the normal installed-wheel test suite runs all six.

| Fixture | Combined behavior | Semantic and visual checks |
|---|---|---|
| `Invoice` | Styled report heading, fixed table, item/amount rows, borders and colored cells | Each item appears once in DOCX and PDF; visible fills survive export |
| `Merged` | Horizontal header merge above ordinary cells | Saved semantic column span is two; header text appears once; fill remains visible |
| `Nested` | Nested table followed by a paragraph inside an outer cell | All inner/outer labels appear once; nested green fill survives DOCX/PDF rendering |
| `Repeated` | Long table, repeated header, fixed minimum row heights and intact rows | Multiple pages; body rows appear once; header appears once per rendered PDF page |
| `RTL` | Logical cell order with right-to-left table layout | DOCX semantic rows retain logical order; native/PDF red and blue cells visibly reverse horizontal order |
| `Pictures` | Inline image followed by text inside a colored cell | Text remains once; actual blue source-image pixels survive native/PDF output |

Every fixture is serialized as DOCX and exported to PDF, and both outputs reopen. The test compares page counts and renders every page at 72 DPI. It checks representative source colors rather than treating a valid PNG signature as proof of visible content. Repeated headers are intentionally repeated in rendered PDF content; the DOCX semantic source contains the header once.

This completes TBL12's named fixture pack. It does not establish Microsoft Word-identical pagination, arbitrary imported-document preservation, vertical-merge overflow handling, patterned fill fidelity or tagged PDF accessibility. Those remain separate roadmap criteria. The broader table tests cover mutation rollback, inheritance, nested width limits, row splitting and ownership.


## Cell padding and alignment matrix

The additional [test_cell_layout_matrix.py](../tests/test_cell_layout_matrix.py) checks existing cell formatting without adding an API. Its six 3×3 matrices combine horizontal/down/up text orientation with ordinary or RTL table/paragraph direction. Each matrix crosses all three vertical alignments with left/center/right paragraph alignment, asymmetric cell padding, fixed fitting rows, distinct fills and visible rules.

Every source label must survive once in logical DOCX rows and reopened PDF extraction. Saved native XML must retain the actual padding, alignment and orientation properties. At72/144/300 DPI, checks locate visible cell fills and red glyphs, require text to stay inside padded frames, verify independent movement along/across the writing direction and compare native/PDF glyph/frame bounds within two pixels. The fill measurement accounts for the declared 1 pt borders; it does not assume the colored interior is the complete geometric cell. Native row layout includes padding/border contributions. For this fixture's uniform 12 pt bottom padding, a requested 100 pt exact row produces a 112 pt native row step and a 111 pt solid interior after the border. Both absolute dimensions are checked within two raster pixels, separately from source/PDF parity; no Python layout implementation is introduced.

A separate paragraph-direction control keeps table RTL and column identities fixed while toggling the inline start edge. Another fixture preserves nine truly empty cells, their source paragraphs, padding/orientation/vertical alignment and visible fills without inventing text. An independent copy gains short labels without changing those frames; resetting direct formatting restores inheritance. An attempted inter-cell-spacing key is rejected atomically.

This completes the local combined acceptance slice of TBL8, not the full criterion. Inter-cell spacing still lacks a native model/read/write/layout path at the pinned revision. Quarter-turned Latin labels do not prove East Asian vertical text, arbitrary script/font behavior, Word-identical pagination, imported preservation or broader shared-edge rendering fidelity.
