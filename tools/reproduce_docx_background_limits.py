"""Run with an installed doc_ex wheel and tests on PYTHONPATH; no Cargo needed.

Original synthetic fixtures. At WordCraft f2e15cf, show watermark ink bounds/color
and excessive-tile background mismatch in native DOCX versus exported PDF.
"""
from collections import Counter
from io import BytesIO
import re
import struct
from zipfile import ZipFile, ZIP_DEFLATED
import zlib

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def archive(parts):
    output = BytesIO()
    with ZipFile(output, 'w', ZIP_DEFLATED) as z:
        for name, data in parts.items():
            z.writestr(name, data)
    return output.getvalue()


def parts(data):
    with ZipFile(BytesIO(data)) as z:
        return {name: z.read(name) for name in z.namelist()}


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    row = bytes((255, 0, 0, 255, 0, 0, 255, 255))
    return (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress((b'\0' + row) * 2)) + chunk(b'IEND', b''))


def main():
    with doc_ex.new('docx') as document:
        document.sections[0].page_size = (400, 500)
        document.sections[0].header().add_paragraph('')
        files = parts(document.to_bytes())
    header = next(name for name in files if name.startswith('word/header') and name.endswith('.xml'))
    shape = (b'<w:r><w:pict><v:shape id="PowerPlusWaterMarkObject1" '
             b'style="position:absolute;mso-position-horizontal:center;mso-position-vertical:center" '
             b'fillcolor="#FF0000"><v:fill opacity=".5"/><v:textpath on="t" '
             b'style="font-family:Arial" string="DRAFT"/></v:shape></w:pict></w:r>')
    files[header] = files[header].replace(b'</w:p>', shape + b'</w:p>', 1)
    with doc_ex.open(archive(files)) as source, doc_ex.open(source.convert('pdf')) as pdf:
        for kind, document in [('DOCX', source), ('PDF', pdf)]:
            width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=72)))
            coords = [(i // 4 % width, i // 4 // width) for i in range(0, len(pixels), 4)
                      if pixels[i] > 240 and pixels[i + 1] < 220 and pixels[i + 2] < 220]
            colors = Counter(tuple(pixels[i:i + 4]) for i in range(0, len(pixels), 4)
                             if pixels[i] > 240 and pixels[i + 1] < 220)
            bounds = tuple(operation(c[axis] for c in coords)
                           for axis, operation in [(0, min), (1, min), (0, max), (1, max)])
            print('watermark', kind, bounds, colors.most_common(1))

    with doc_ex.new('docx') as document:
        document.sections[0].page_size = (240, 320)
        document.add_image(png(), width=10)
        files = parts(document.to_bytes())
    rid = re.search(rb'r:embed="([^"]+)"', files['word/document.xml']).group(1)
    background = (b'<w:background><v:background><v:fill r:id="' + rid
                  + b'" type="tile"/></v:background></w:background>')
    files['word/document.xml'] = files['word/document.xml'].replace(b'<w:body>', background + b'<w:body>', 1)
    with doc_ex.open(archive(files)) as source, doc_ex.open(source.convert('pdf')) as pdf:
        for kind, document in [('DOCX', source), ('PDF', pdf)]:
            width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=144)))
            print('tile', kind, [tuple(pixels[(100 * width + x) * 4:(100 * width + x) * 4 + 4])
                                 for x in (10, 13, 16, 19, 22, 25)])


if __name__ == "__main__":
    main()
