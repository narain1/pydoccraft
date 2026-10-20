"""Reproduce pinned PdfCraft annotation-inventory limits with synthetic PDFs.

Run with an installed doc_ex wheel: python tools/reproduce_pdf_annotation_inventory_limits.py
Importing this module only defines functions; it does not open files, import the
native extension or run a probe. Circle is the native PDF subtype for the public
ellipse kind. These are test fixtures, not a production document writer.

The tiny JSON results do not measure peak memory. Allocation and truncated-quad
claims come from the pinned source audit in docs/upstream-roadmap.md.
"""
import json


def fixture(*, colors=3, quads=0, repeats=1):
    """Return a deterministic one-page PDF with one indirect ellipse dictionary."""
    if any(type(value) is not int for value in (colors, quads, repeats)):
        raise TypeError("fixture counts must be integers")
    # Arrays and repeated references remain small enough for an ordinary regression.
    if not (0 <= colors <= 100_000 and 0 <= quads <= 8193 and 1 <= repeats <= 3):
        raise ValueError('probe bounds exceeded')
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>',
               b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
               b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Annots [' + b'4 0 R ' * repeats + b'] >>',
               b'<< /Type /Annot /Subtype /Circle /Rect [20 20 140 80] /C [' + b'0 ' * colors +
               b'] /BS << /W 2 /S /S >> /Contents (probe) ' +
               (b'/QuadPoints [' + b'0 ' * quads + b'] ' if quads else b'') + b'>>']
    output = bytearray(b'%PDF-1.7\n')
    offsets = []
    for number, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f'{number} 0 obj\n'.encode() + value + b'\nendobj\n')
    xref = len(output)
    output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        output.extend(f'{offset:010d} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output)


def probe():
    """Return bounded observations; public errors stay visible in the report."""
    import doc_ex

    results = []
    for name, kwargs in [('oversized_color', {'colors':100_000}),
                         ('quad_tail', {'quads':8193}),
                         ('repeated_reference', {'repeats':3})]:
        data = fixture(**kwargs)
        with doc_ex.open(data) as source, source.edit() as edited:
            # This existing private boundary calls the public native summaries().
            payload = edited._native.annotations(0)
            entries = json.loads(payload)
            result = {'case':name, 'input_bytes':len(data), 'summary_json_bytes':len(payload),
                      'returned_count':len(entries), 'first_color':entries[0]['color']}
            try:
                edited.pages[0].annotations[0].color = 'blue'
                result['update'] = 'accepted'
            except Exception as error:
                result['update'] = type(error).__name__
            results.append(result)
    return results


if __name__ == '__main__':
    print(json.dumps(probe(), indent=2))
