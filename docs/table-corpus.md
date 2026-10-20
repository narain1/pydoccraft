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
