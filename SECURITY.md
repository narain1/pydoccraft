# Security policy

pydoccraft processes untrusted PDF, DOCX, and PPTX data in-process. Security reports are welcome and should be handled privately.

## Supported versions

Security fixes are provided for the latest released version. Older releases may receive a fix when practical, but users should normally upgrade to the newest release.

## Reporting a vulnerability

Use GitHub's **Report a vulnerability** option in the repository Security tab to open a private security advisory. Do not report vulnerabilities in public issues, discussions, pull requests, or document attachments.

Include, when safe:

- the affected pydoccraft version and platform;
- the source format and operation involved;
- impact and realistic attack scenario;
- minimal reproduction steps;
- a small synthetic reproducer, or instructions for generating one;
- any known workaround.

Do not send confidential documents, personal data, credentials, or third-party files you cannot redistribute. If a malicious sample is necessary, coordinate privately before attaching it.

The maintainer aims to acknowledge a report within 7 days and provide an initial assessment within 14 days. Resolution timing depends on severity and whether a fix is required in a pinned upstream engine. Please allow coordinated remediation before public disclosure.

## Scope

Relevant reports include parser crashes with security impact, memory-safety issues, archive traversal or expansion bypasses, unexpected filesystem/network/process access, resource-limit bypasses, and malicious output generation. General bugs and unsupported fidelity cases belong in the public issue tracker.
