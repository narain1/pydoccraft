# Documentation website

The project documentation is published at [narain1.github.io/pydoccraft](https://narain1.github.io/pydoccraft/). It follows the reviewed `main` branch. Development and unreleased features remain labeled in the guides; the site is not evidence that a feature ships in the latest package release.

## Edit and preview

Keep API and support details in the existing `docs/*.md` files. The builder renders those files directly; do not maintain a second copy for the website. The landing page and presentation assets live in `website/`. The root README, roadmap, contribution guide, security policy and support guide are also included. Source-code links and other repository files point back to GitHub.

```console
python -m venv /tmp/pydoccraft-docs
/tmp/pydoccraft-docs/bin/python -m pip install --require-hashes -r website/requirements.txt
/tmp/pydoccraft-docs/bin/python tools/build_site.py
python tools/check_site.py build/site
python -m http.server 8000 --directory build/site
```

Open `http://localhost:8000/`. The generated site uses relative links, so the same files work under the GitHub project path `/pydoccraft/`. Output is ignored under `build/site/`; commit only the Markdown, builder and assets. When removing a source page, clean `build/site/` before rebuilding so an obsolete generated page is not left in a local preview. CI always starts with an empty output directory.

The sole additional dependency is hash-pinned Python-Markdown 3.7 (BSD-3-Clause), used only while building documentation. Neither the website nor the package needs a JavaScript framework, CDN, font download, analytics service or runtime network access. The build does not import `doc_ex`, load its native extension or run Cargo. Its generated pages contain no JavaScript.

`tools/check_site.py` validates every generated local link, asset path and fragment, including the navigation and table of contents. The builder also rejects missing source-relative repository targets. External URLs are not fetched by the check. API examples are maintained with the existing package tests and cookbook validation; the site builder does not execute them.

## GitHub Pages deployment

The [Documentation workflow](../.github/workflows/docs.yml) builds and checks every pull request with a read-only token. It deploys only a successful build on `main`, including a manually dispatched run from `main`. Pull requests cannot publish. The deploy job alone receives `pages: write` and `id-token: write`; it uses the `github-pages` environment and pinned official GitHub actions.

Before the first deployment, configure **Settings → Pages → Build and deployment → Source → GitHub Actions**. Keep the `github-pages` environment restricted to `main`. The repository must permit GitHub Pages and Actions. No personal access token or custom-domain secret is required.

After a reviewed merge, inspect the workflow's deployment URL and open the home page, the API guide, an authoring guide and the roadmap at the public project path. Check a narrow/mobile viewport as well. A green build is local content/link evidence; a successful deployment and reachable public URL establish that hosting works.

The workflow follows GitHub's [custom Pages workflow documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages). Enabling Pages is a repository setting; the workflow does not grant itself permission to enable the service.
