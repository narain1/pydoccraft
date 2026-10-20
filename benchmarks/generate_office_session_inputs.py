"""Generate synthetic fixtures with the installed pydoccraft native writers."""
from pathlib import Path
import argparse
import struct,zlib
def chunk(tag,data):
    return struct.pack('>I',len(data))+tag+data+struct.pack('>I',zlib.crc32(tag+data)&0xffffffff)
PNG=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',64,64,8,6,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+b'\xff\0\0\xff'*64)*64))+chunk(b'IEND',b'')
import doc_ex
parser = argparse.ArgumentParser(description="Generate the original 20-page DOCX/PPTX session benchmark inputs.")
parser.add_argument("output", type=Path, help="new output directory; existing directories are rejected")
args = parser.parse_args()
out = args.output
out.mkdir(parents=True, exist_ok=False)
with doc_ex.new('docx',size=(612,792)) as document:
    document.sections[0].margins=36
    for page in range(20):
        if page: document.add_page_break()
        document.add_paragraph(f'Synthetic report page {page+1}',style='Heading 1')
        for row in range(12):
            document.add_paragraph(f'Row {row+1}: bounded native layout, shared rendering and retained document state.')
        document.add_table([['Item','Count','Unit','Total']]+[[f'Item {n}','2','4.50','9.00'] for n in range(4)])
        document.add_image(PNG,width=24,height=24)
    document.save(out/'report.docx')
with doc_ex.new('pptx',size=(720,540)) as document:
    for number in range(20):
        slide=document.slides.add()
        slide.add_text(f'Synthetic slide {number+1}',box=(30,20,640,50),font_size=24)
        for row in range(8):
            slide.add_text(f'Row {row+1}: shared native resources and model reuse',box=(30,80+row*30,640,28),font_size=14)
        slide.add_table([['Item','Count'],['A','20'],['B','30']],box=(30,350,500,120),font_size=14)
        slide.add_image(PNG,box=(570,360,60,60))
    document.save(out/'slides.pptx')
for path in out.iterdir():
    with doc_ex.open(path) as document: print(path.name,path.stat().st_size,len(document))
