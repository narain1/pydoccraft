"""Synthetic imported-size DOCX fixtures, shared by the default-size tests.

The test that compared an imported file with an omitted default size against explicit sizes is
set aside until WordCraft's reader and resolver agree on that size (see the validation log).
"""

from fixtures import package

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
LABEL = "SIZEKEEP"
URI = "https://example.test/default-size"


def imported_size(size: int | None, *, styles: bool = True) -> bytes:
    """Synthetic OOXML, not runtime format logic or third-party document bytes."""
    relationships = f'<Relationship Id="link" Type="{R}/hyperlink" Target="{URI}" TargetMode="External"/>'
    if styles:
        relationships += f'<Relationship Id="styles" Type="{R}/styles" Target="styles.xml"/>'
    members = {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "_rels/.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>',
        "word/_rels/document.xml.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationships}</Relationships>',
        "word/document.xml": f'''<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>
<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/><w:ind w:left="0" w:right="0" w:firstLine="0"/></w:pPr>
<w:hyperlink r:id="link"><w:r><w:rPr><w:rFonts w:ascii="DejaVu Sans" w:hAnsi="DejaVu Sans"/><w:b/><w:color w:val="FF0000"/><w:kern w:val="3276"/><w:u w:val="none"/></w:rPr><w:t>{LABEL}</w:t></w:r></w:hyperlink></w:p>
<w:sectPr><w:pgSz w:w="3200" w:h="1200"/><w:pgMar w:top="240" w:bottom="240" w:left="240" w:right="240"/></w:sectPr>
</w:body></w:document>''',
    }
    if styles:
        ordinary = "" if size is None else f'<w:sz w:val="{size * 2}"/>'
        members["word/styles.xml"] = f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr>{ordinary}</w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style></w:styles>'
    return package(members)
