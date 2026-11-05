# Security

Please report a vulnerability privately, through
[GitHub's private vulnerability reporting](https://github.com/jrimmer/ofolder/security/advisories/new),
not in a public issue. Say what you found and how to reproduce it; you will get an answer, and a fix
and a note in the changelog if it is real.

What ofolder does that is worth a careful look: it runs as you and reads and drives file dialogs
from outside (accessibility tree, window manager commands and key events sent to a verified window),
it registers itself as a file-chooser backend, it decodes the pictures and videos you select, and its
installer and updater download and run a program. [docs/review-2026-11-03.md](docs/review-2026-11-03.md)
records what was checked and what is known to be open.

Only the latest release is supported.
