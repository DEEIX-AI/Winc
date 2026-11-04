# Security Policy

## Reporting a Vulnerability

Please do not report security vulnerabilities in public issues, pull requests, or discussions.

Report them privately, either through GitHub's
[private vulnerability reporting](https://github.com/DEEIX-AI/Winc/security/advisories/new) or by
email to support@winc.si. Please include:

- a description of the problem and its impact
- the affected version and platform
- steps to reproduce it, or a proof of concept
- any relevant logs or screenshots

We will confirm receipt, keep you informed while we work on a fix, and credit you in the release
notes unless you prefer otherwise. Please give us a reasonable time to release a fix before
disclosing the problem publicly.

## Scope

Reports are in scope when they affect winc itself, for example:

- the in-app updater: signature verification, the update manifests, or how updates are installed
- opening files or folders: parsing, previews, and anything that can execute code without the user
  asking
- the integrated terminal and the pseudo-terminal helper process
- the release packages and their signatures

Out of scope:

- problems that require an attacker to already control the user's account or machine
- vulnerabilities in third-party tools that winc runs on the user's behalf, such as Git or code
  formatters, unless winc invokes them unsafely
- denial of service by opening a deliberately huge or malformed file, unless it leads to more than
  a crash

## Supported Versions

Security fixes go into the latest release. Update to the newest version through the in-app updater
or from [winc.si](https://winc.si/releases/).
