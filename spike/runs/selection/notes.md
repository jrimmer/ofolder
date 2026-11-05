# Selection spike, 2026-11-03

Scratch folder with sub-folder, aether.png, alpha.txt, archlinux-logo.png, beta.txt and others.

## Flea (Open dialog through the portal, then the maintainer's open Save dialog read-only)

    qs ipc --pid <picker> show            -> target fleapicker: path, cursorName, cursor, marks, markedUris, snapshot, ...
    path / cursorName / marks, 10 calls each: 17.9 / 18.4 / 17.7 ms per call; snapshot 18.2 ms
    Down (sent only after checking the picker was the active window): aether.png -> alpha.txt, seen by the next call, 20 ms
    selection_probe flea <pid>:
      selection .../pick/sub-folder (31 ms)
      Down -> .../pick/aether.png (21 ms)
      Down -> .../pick/alpha.txt (20 ms)
      a Save dialog at home: /home/user/project (20 to 26 ms)
    read_folder in the scratch folder: None. The --backend helpers held no directory fd there.
    AT-SPI: application 'qs' registered with 0 children; org.a11y.Status IsEnabled=false.

## GTK (OpenFile on org.freedesktop.impl.portal.desktop.gtk directly)

    table 'Files' [focused] kids=48: 4 column headers, then 4 cells per row; first row selected at open.
    Python Atspi.Table.get_selected_rows -> [0], cell (0,0) children ['', 'gamma.md'], 0.7 ms
    Down keys to the verified-active dialog: selection stayed on row 0 (not explained; the dialog was
    covered by another floating window at the time).
    selection_probe gtk: folder .../pick (49 to 53 ms), selection .../pick/gamma.md (138 to 160 ms)
    Closed by the Cancel button's action: REPLY (1, {...})

## Strata (main window on the scratch folder, not the chooser)

    list 'pick' kids=3: list item 'sub-folder' [selected,focused] (initial)
    Down -> aether.png (52 ms read), Down -> alpha.txt (37 ms read)
    The chooser was not run: its portal toggle rewrites the portal config the maintainer's ofolder install uses.
