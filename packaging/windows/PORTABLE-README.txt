Dmux portable build
===================

Dmux proves that a PostgreSQL migration copied every row and value.
This folder runs without installing anything and without administrator
rights.

To start: double-click dmux-app.exe.

What the PC needs
-----------------
* Windows 10 or 11, 64-bit.
* Microsoft Edge WebView2 Runtime. It is already present on Windows 11 and on
  most Windows 10 PCs that have Edge. This portable build does NOT carry it:
  if Dmux does not open, install the WebView2 Runtime first (the "offline
  setup" download on the Dmux release page carries it, for PCs with no
  internet).

What is in this folder
----------------------
* dmux-app.exe          the Dmux desktop app
* python\               a private copy of Python, used only for custom
                        checks (hooks). Nothing is installed system-wide.
* THIRD_PARTY_NOTICES.md and OFL-1.1.txt   licences of bundled software

Keep the folder together: dmux-app.exe finds python\ next to itself.
Dmux keeps its saved connections and run history in your Windows user
profile, not in this folder. Passwords are never saved in a file.

Before you run it, check the download against the SHA256SUMS file on the
release page (see docs/guide/verify-download.md). The build is not yet
code-signed, so Windows SmartScreen may warn.
