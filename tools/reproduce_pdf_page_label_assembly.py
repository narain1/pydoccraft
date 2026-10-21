"""Show native display-label behavior under existing PDF assembly operations.

Run with an installed wheel; importing this module performs no document work and
requires no native extension. The synthetic four-page source is test input only.
"""
import json


def fixture():
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums "
        b"[0 << /S /r >> 2 << /S /D /P (Body-) /St 5 >>] >> >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R] /Count 4 >>",
    ]
    for width in (200, 210, 220, 230):
        objects.append(
            (f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} 200] "
             "/Resources << >> >>").encode()
        )
    output = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode() + value + b"\nendobj\n")
    xref = len(output)
    output.extend(b"xref\n0 7\n0000000000 65535 f \n")
    for offset in offsets:
        output.extend(f"{offset:010d} 00000 n \n".encode())
    output.extend(
        f"trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(output)


def snapshot(data):
    import doc_ex

    with doc_ex.open(data) as document:
        return [(page.metadata["width"], page.metadata["label"]) for page in document]


def probe():
    import doc_ex

    data = fixture()
    results = {"source": snapshot(data)}
    with doc_ex.open(data) as source, source.edit() as edited:
        with edited.copy() as copied:
            results["full_copy"] = snapshot(copied.to_bytes())
        edited.pages.move(3, 0)
        results["move_3_to_0"] = snapshot(edited.to_bytes())
    with doc_ex.open(data) as source, source.edit() as edited:
        del edited.pages[0]
        results["delete_0"] = snapshot(edited.to_bytes())
    with doc_ex.open(data) as source, source.edit() as edited:
        with edited.copy(pages=[3, 0]) as selected:
            results["subset_3_0"] = snapshot(selected.to_bytes())
    with doc_ex.open(data) as source, doc_ex.new("pdf") as target:
        target.pages.insert(1, source, pages=[3, 0])
        results["import_3_0"] = snapshot(target.to_bytes())
    return results


if __name__ == "__main__":
    print(json.dumps(probe(), indent=2))
