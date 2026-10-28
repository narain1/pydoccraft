# Installation and deployment

Install the distribution named `pydoccraft`; its Python import is `doc_ex`:

```console
python -m pip install pydoccraft
```

```python
import doc_ex
print(doc_ex.__version__)
```

This website follows the development branch. The installed version determines
which features are available. For release-specific documentation, choose the
matching tag from [GitHub releases](https://github.com/narain1/pydoccraft/releases)
and read that tag's README and `docs/` directory. EPUB support and page HTML
extraction shown on this development site are unreleased. The authoring APIs for
PDF, DOCX and PPTX were introduced in 0.3.0. Check the release notes for the
particular operation before relying on it.

## Python and platforms

The package metadata requires Python 3.11 or newer. The release workflow builds
and validates Python 3.11, 3.12 and 3.13 wheels on Linux, macOS and Windows
runners. This is a build matrix, not a guarantee that every OS/CPU/Python
combination has a published wheel. Check the files for the specific version on
[PyPI](https://pypi.org/project/pydoccraft/#files) and the release's validation
artifacts. No cross-platform support is inferred from a local successful build.

To require a compatible prebuilt wheel and fail rather than building from source:

```console
python -m pip install --only-binary=:all: pydoccraft
```

If no compatible wheel is available, choose a supported environment or follow
[development and source-build instructions](development.md). A source build
requires Rust 1.95, Maturin and access to pinned build dependencies. Runtime
document processing needs no network, office suite, external executable or
cloud credentials. The wheel contains the extension and required bundled assets.

## Smoke test

With a local, text-containing PDF named `sample.pdf`:

```python
from pathlib import Path
import doc_ex

with doc_ex.open("sample.pdf") as document:
    print("Pages:", document.page_count)
    print(document[0].get_text()[:300])
    Path("sample-page.png").write_bytes(document.render_page(0, dpi=96))
```

A scanned PDF without a text layer may produce an empty string; it can still
render. See [troubleshooting](troubleshooting.md) for other common symptoms.

## Containers, memory and concurrency

Install a wheel matching the container's OS, architecture, Python ABI and libc;
run the smoke test in the final image. Provide inputs in the final environment. Rendering depends on font availability
and substitutions can change layout; test representative documents in the actual
environment. The package includes required assets, not every font used by
arbitrary documents. General application font discovery and configuration are
not established uniformly across formats; placing a font in a container does
not guarantee that an engine will use it. See [formats and rendering](formats.md) and
[dependency attribution](dependencies-and-licenses.md).

Document parsing is eager and image-heavy files can use much more memory than
the input. Apply upload-size, execution-time and worker-memory limits; consider
isolated worker processes for untrusted documents and tune concurrency against
measured peak memory. Existing render-session measurements report that calls
hold the Python GIL. A general same-document thread-safety or concurrent mutation
contract is not established; avoid sharing mutable handles across threads and
do not assume native calls run in parallel. Process isolation also helps bound
failures, but it is an application deployment choice, not a package sandbox.
See [safety and resource behavior](safety.md).
