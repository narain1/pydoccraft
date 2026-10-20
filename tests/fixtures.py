from __future__ import annotations

import base64
import io
import zipfile

PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z1ioAAAAASUVORK5CYII="
)
RGB_PIXEL = b"\xff\x00\x00"


def package(members: dict[str, bytes | str], compression=zipfile.ZIP_DEFLATED) -> bytes:
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=compression) as archive:
        for name, value in members.items():
            archive.writestr(name, value.encode() if isinstance(value, str) else value)
    return output.getvalue()


def docx_fixture() -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Target="word/document.xml" Type="officeDocument"/></Relationships>',
        "word/document.xml": '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body><w:p><w:pPr><w:pStyle w:val="CustomHeading"/></w:pPr><w:r><w:t>Example heading</w:t></w:r></w:p><w:p><w:commentRangeStart w:id="0"/><w:hyperlink r:id="rId1"><w:r><w:t>Example link</w:t></w:r></w:hyperlink><w:r><w:t> and </w:t></w:r><w:hyperlink r:id="rId2"><w:r><w:t>Example link</w:t></w:r></w:hyperlink><w:r><w:drawing><wp:inline><wp:extent cx="9525" cy="9525"/><wp:docPr id="1" name="image1.png" descr="Red pixel"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="image1.png" descr="Red pixel"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rImage1"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9525" cy="9525"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Score</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>Alice</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>91</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:br w:type="page"/></w:r></w:p><w:p><w:r><w:t>Second page</w:t></w:r></w:p></w:body></w:document>',
        "word/_rels/document.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="https://example.com" Type="hyperlink" TargetMode="External"/><Relationship Id="rId2" Target="https://example.org" Type="hyperlink" TargetMode="External"/><Relationship Id="rImage1" Target="media/image1.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/><Relationship Id="rStyles" Target="styles.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"/><Relationship Id="rComments" Target="comments.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments"/><Relationship Id="rCommentsEx" Target="commentsExtended.xml" Type="http://schemas.microsoft.com/office/2011/relationships/commentsExtended"/></Relationships>',
        "word/styles.xml": '<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="CustomHeading"><w:name w:val="Audit heading"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style></w:styles>',
        "word/comments.xml": '<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:comment w:id="0" w:author="Ann" w:initials="A"><w:p w14:paraId="11111111"><w:r><w:t>Review this</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ben" w:initials="B"><w:p w14:paraId="22222222"><w:r><w:t>Reply here</w:t></w:r></w:p></w:comment></w:comments>',
        "word/commentsExtended.xml": '<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="11111111" w15:done="0"/><w15:commentEx w15:paraId="22222222" w15:paraIdParent="11111111" w15:done="0"/></w15:commentsEx>',
        "word/media/image1.png": PNG,
        "word/media/unreferenced.png": PNG,
    })


def pptx_fixture() -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Target="ppt/presentation.xml" Type="officeDocument"/></Relationships>',
        "ppt/presentation.xml": '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="5143500"/></p:presentation>',
        "ppt/_rels/presentation.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="slides/slide1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"/></Relationships>',
        "ppt/slides/slide1.xml": '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="457200" y="457200"/><a:ext cx="5486400" cy="1371600"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr><a:hlinkClick r:id="h1"/></a:rPr><a:t>Repeat</a:t></a:r><a:r><a:t> middle </a:t></a:r><a:r><a:rPr><a:hlinkClick r:id="h2"/></a:rPr><a:t>Repeat</a:t></a:r><a:br/><a:r><a:t>line two</a:t></a:r></a:p><a:p><a:r><a:t>Second paragraph</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="3" name="image1.png" descr="Red pixel"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="i"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="6400800" y="457200"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="457200" y="2286000"/><a:ext cx="5486400" cy="1828800"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1"/><a:tblGrid><a:gridCol w="2743200"/><a:gridCol w="2743200"/></a:tblGrid><a:tr h="914400"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Key</a:t></a:r></a:p><a:p><a:r><a:t>detail</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Value</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr><a:tr h="914400"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>A</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>B</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>',
        "ppt/slides/_rels/slide1.xml.rels": '<Relationships xmlns="x"><Relationship Id="h1" Target="https://example.com/one" TargetMode="External" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"/><Relationship Id="h2" Target="https://example.com/two" TargetMode="External" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"/><Relationship Id="unused" Target="https://example.com/unused" TargetMode="External" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"/><Relationship Id="i" Target="../media/image1.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/><Relationship Id="n" Target="../notesSlides/notesSlide1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide"/><Relationship Id="c" Target="../comments/comment1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments"/></Relationships>',
        "ppt/media/image1.png": PNG,
        "ppt/notesSlides/notesSlide1.xml": '<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes"/><p:cNvSpPr txBox="1"/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Speaker</a:t></a:r><a:br/><a:r><a:t>notes</a:t></a:r></a:p><a:p><a:r><a:t>Next note</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>',
        "ppt/comments/comment1.xml": '<p:cmLst xmlns:p="p"><p:cm><p:text>Check slide</p:text></p:cm></p:cmLst>',
    })


def pdf_fixture() -> bytes:
    content1 = b"BT /F1 12 Tf 1 0 0 1 72 720 Tm (Page one) Tj 1 0 0 1 72 680 Tm (Name) Tj 1 0 0 1 180 680 Tm (Score) Tj 1 0 0 1 72 660 Tm (Alice) Tj 1 0 0 1 180 660 Tm (91) Tj ET"
    content2 = b"BT /F1 12 Tf 72 720 Td (Second page image) Tj ET q 20 0 0 20 72 650 cm /Im1 Do Q"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R /Annots [7 0 R] >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 9 0 R >> >> /Contents 10 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        f"<< /Length {len(content1)} >>\nstream\n".encode() + content1 + b"\nendstream",
        b"<< /Type /Annot /Subtype /Link /Rect [72 700 150 720] /A 8 0 R >>",
        b"<< /S /URI /URI (https://example.com) >>",
        b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n" + RGB_PIXEL + b"\nendstream",
        f"<< /Length {len(content2)} >>\nstream\n".encode() + content2 + b"\nendstream",
    ]
    output = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode())
    output.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(output)


def inline_break_docx_fixture(before="A", after="B") -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "word/document.xml": f'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{before}</w:t><w:br w:type="page"/><w:t>{after}</w:t></w:r></w:p></w:body></w:document>',
    })


def three_page_docx_fixture() -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "word/document.xml": '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>A</w:t><w:br w:type="page"/><w:br w:type="page"/><w:t>B</w:t></w:r></w:p></w:body></w:document>',
    })


def inherited_resources_pdf_fixture() -> bytes:
    content = b"BT /F1 12 Tf 72 720 Td (Inherited) Tj ET q 20 0 0 20 72 650 cm /InheritedImage Do Q"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 4 0 R >> /XObject << /InheritedImage 6 0 R >> >> >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream",
        b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n" + RGB_PIXEL + b"\nendstream",
    ]
    output = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode())
    output.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(output)


def nested_table_and_break_before_docx_fixture() -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Target="word/document.xml" Type="officeDocument"/></Relationships>',
        "word/document.xml": '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Outer</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Inner</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl><w:p><w:pPr><w:pStyle w:val="BreakBefore"/></w:pPr><w:r><w:t>After break</w:t></w:r></w:p></w:body></w:document>',
        "word/_rels/document.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Target="styles.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"/></Relationships>',
        "word/styles.xml": '<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="BreakBefore"><w:name w:val="Break before"/><w:basedOn w:val="Normal"/><w:pPr><w:pageBreakBefore/></w:pPr></w:style></w:styles>',
    })