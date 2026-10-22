# diodon-next: implementation plan

A personal clipboard manager for X11 (Cinnamon). Stores history in SQLite,
shows it in a native GTK menu opened by a global shortcut, and pastes the
chosen entry into the window that had focus. Plain text today; rich text and
images are planned (Phase 6).

Not distributed for now (a `.deb` may come later, and the project will be
renamed, see Phase 9). Optimise for "works on this machine" and few
dependencies.

## Decisions

| Topic | Choice | Reason |
|---|---|---|
| Language | Rust (2024 edition) | User preference |
| UI / clipboard | `gtk` 0.18 (GTK3 bindings) | Native menu, clipboard `owner-change` signal, themed like Cinnamon |
| Storage | SQLite via `rusqlite` with the `bundled` feature | Compiles SQLite from C (gcc is already installed), so no `libsqlite3-dev` is needed |
| Paste / focus | `xdotool` (subprocess) | User preference, simplest |
| Global shortcut | Cinnamon custom keybinding running `diodon-next show` | No code, no X key grabbing |
| Process model | Single-instance `gtk::Application` (D-Bus activation) | The `show` invocation activates the daemon, so no custom IPC |
| Content | Plain text today; rich text (HTML, RTF) and images in Phase 6 | v1 scope was text only |
| Platform | X11 only | Current session is X11; Wayland would need a different design |

Dependencies in `Cargo.toml`: `gtk`, `rusqlite` (features: `bundled`). Nothing
else, and Phase 6 and 7 add no crates: hashing uses `glib::Checksum`, the one
`unsafe` clipboard call goes through the `gtk::ffi` re-export, image work uses
`gdk_pixbuf` (part of GTK). Timestamps use `std::time`. Argument parsing uses
`std::env::args`.

System packages to install (needs approval): `libgtk-3-dev`, `pkg-config`,
`xdotool`. Toolchain: `rustup` (stable).

## Architecture

One binary, two subcommands.

- `diodon-next daemon`: started at login. Owns the clipboard watcher, the
  database and the menu.
- `diodon-next show`: bound to the global shortcut. Forwards to the running
  daemon (over D-Bus) and returns immediately. If no daemon is running, this
  process becomes the primary instance, shows the menu, and **exits once the
  interaction is over** (menu dismissed, or paste sent plus a 500 ms grace
  period so the target can fetch the text). It does not linger as a hidden
  daemon, and it records nothing after it exits.
- `diodon-next quit`: stops the running daemon.

This mirrors the original Diodon, which runs as an autostarted daemon and
whose shortcut just runs `diodon` to activate it.

Both use the same `gtk::Application` ID (e.g. `dev.quentin.DiodonNext`), so
GTK routes the second invocation to the first process.

```
src/
  main.rs        arg parsing, Application setup, actions (show, quit)
  db.rs          SQLite schema and queries
  watcher.rs     clipboard owner-change handling
  menu.rs        popup menu construction and positioning
  paste.rs       xdotool wrappers: active window, refocus, send keys
```

## Database

This section describes the **v1 schema, which is in use today**. Phase 6
replaces it with a v2 schema (hash-keyed, multi-format, storage budget).

File: `$XDG_DATA_HOME/diodon-next/history.db` (default
`~/.local/share/diodon-next/history.db`). The directory is created with
mode 0700 and the file is chmod 0600, since it may contain secrets.

```sql
PRAGMA journal_mode = WAL;
PRAGMA user_version = 1;

CREATE TABLE IF NOT EXISTS entries (
  id         INTEGER PRIMARY KEY,
  content    TEXT    NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,   -- unix seconds, first time seen
  used_at    INTEGER NOT NULL,   -- unix seconds, last copied or pasted
  pinned     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS entries_used_at ON entries(used_at DESC);
```

Operations in `db.rs`:

- `open()`: create the directory and file, apply the pragmas, and run the
  migration keyed on `user_version`.
- `upsert(content)`: `INSERT ... ON CONFLICT(content) DO UPDATE SET used_at = excluded.used_at`.
  This is the dedupe: re-copying an old entry moves it to the top.
- `recent(limit)`: returns every pinned entry plus the `limit` newest unpinned
  ones, ordered `pinned DESC, used_at DESC, id DESC`. Each `Entry` carries
  `id`, `content` and `pinned`. Pinned entries do not count against `limit`.
- `prune(max)`: delete unpinned rows beyond the newest `max` by `used_at`.
  Run after every insert.
- `clear()`: delete all unpinned rows.
- `toggle_pin(id)`: `UPDATE entries SET pinned = 1 - pinned WHERE id = ?`.
  Re-copying a pinned entry keeps it pinned (the upsert only touches
  `used_at`).

Constants: `MAX_ENTRIES = 50`, `MAX_BYTES = 100_000` (larger text is ignored).
Both become settings in Phase 7, and `MAX_BYTES` disappears in Phase 6: the
storage budget replaces any per-entry limit.

## Phase 0: environment setup

1. Ask for approval, then run:
   `sudo apt install libgtk-3-dev pkg-config xdotool`
2. Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
   (stable toolchain, default profile).
3. In `/home/quentin/dev/diodon-next`: `cargo init --name diodon-next`.
4. Add the dependencies:
   `cargo add gtk@0.18 && cargo add rusqlite --features bundled`.
5. Check `cargo build` succeeds. The first build takes a few minutes.

Done when: an empty `main.rs` builds and links GTK and SQLite.

## Phase 1: de-risk the menu (prototype)

The riskiest part is showing a `gtk::Menu` from a process with no focused
window and having it receive keyboard input and close on outside click.

1. `main.rs`: create the `gtk::Application`, register a `show` `SimpleAction`,
   and hold the application (`app.hold()`) so it stays alive without windows.
2. `menu.rs`: on `show`, read the pointer position with
   `gdk::Display::default().default_seat().pointer().position()`.
3. Create a 1x1 undecorated, transparent, `skip_taskbar` helper `gtk::Window`
   at that position and present it. Anchor the menu to it with
   `menu.popup_at_rect(...)`. If plain `popup_at_pointer(None)` works
   reliably on this system, skip the helper window.
4. Fill the menu with dummy items. Verify all of:
   - arrow keys and Enter work immediately
   - Escape closes it
   - clicking outside closes it
   - digit mnemonics select items
   - the menu appears next to the pointer and stays on-screen on every monitor
   - the helper window is destroyed when the menu hides (`connect_deactivate`)
5. Add the Cinnamon keybinding (see Phase 5) for early testing.

Done when: the shortcut opens a menu that behaves like a normal context menu
every time, from any app.

## Phase 2: clipboard watcher and storage

1. `db.rs`: implement the schema and operations above, with unit tests using
   an in-memory database (`Connection::open_in_memory()`). Test dedupe,
   ordering, pruning and pinned rows surviving the prune.
2. `watcher.rs`:
   - `let cb = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);`
   - `cb.connect_owner_change(...)` then `cb.request_text(...)`.
   - Skip when: text is `None`, empty or whitespace-only, longer than
     `MAX_BYTES`, or equal to the entry we set ourselves (see Phase 3).
   - Skip password-manager entries: request the targets with
     `request_targets` and ignore the change if it lists
     `x-kde-passwordManagerHint`.
   - Otherwise call `db.upsert` and `db.prune`.
3. Share the `Connection` with `Rc<RefCell<...>>`. Everything runs on the GTK
   main thread, so no `Mutex` is needed.
4. Do not watch `PRIMARY`. Highlighting text would flood the history.

Done when: copying in any app adds a row, re-copying moves it to the top, and
the table never exceeds the cap.

## Phase 3: menu from real data, and paste

`show` action flow, in order:

1. Run `xdotool getactivewindow` and store the window ID. Do this before
   anything else, because showing our own UI changes focus.
2. Run `xdotool getwindowclassname <id>` and store the class.
3. Load `db.recent(MAX_ENTRIES)`. If it is empty, show a single disabled
   item "(empty)".
4. Build the menu:
   - Label: first non-empty line, with whitespace runs collapsed, truncated
     to about 60 characters with `...`. Escape `_` as `__` so GTK does not
     treat it as a mnemonic marker.
   - The first 9 items get `_1` through `_9` mnemonics.
   - A separator, then `Clear history`.
5. On item activation, `paste::paste(entry, window_id, class)`:
   1. Record the content as `last_set` so the watcher ignores our own change.
   2. `clipboard.set_text(content)` and `clipboard.store()`.
   3. Make sure the menu and helper window are gone. Wait about 50 ms
      (`glib::timeout_add_local_once`).
   4. `xdotool windowactivate --sync <id>`.
   5. Choose the key combo: `ctrl+shift+v` if the class is in the terminal
      list (`gnome-terminal-server`, `xterm`, `kitty`, `Alacritty`, `tilix`,
      `konsole`, `xfce4-terminal`, `gnome-terminal`, `st`, `urxvt`), otherwise
      `ctrl+v`.
   6. `xdotool key --clearmodifiers <combo>`.
6. Selecting an entry also bumps `used_at` through the normal watcher path.
   With `last_set` suppression, call `db.upsert` directly in `paste` so the
   entry still moves to the top.
7. If the menu is dismissed without a choice, do nothing and do not refocus
   anything.

Failure handling: if `xdotool` is missing or exits non-zero, log to stderr and
leave the content on the clipboard so the user can paste manually.

Done when: the shortcut, a choice, and the text lands in a browser field, a
text editor and a terminal, with the cursor position unchanged.

## Phase 3b: pinning from a submenu

Pinned entries stay at the top of the menu and are never pruned or cleared.
The menu is native `gtk::Menu`, which cannot tell left from right clicks, so
pinning lives in a submenu instead of a click modifier.

Menu layout, top to bottom:

```
1 ★ pinned entry        <- pinned block (all pinned entries)
2 ★ another pinned
-------------------
3 recent entry          <- newest unpinned, up to MAX_ENTRIES
4 older entry
-------------------
Pin / unpin  >          <- submenu, mnemonic _P
Clear history
```

Details:

1. `menu.rs` labels pinned entries with a `★ ` prefix (`entry_label`); Phase 6
   replaces it with the 📌 emoji marker. The
   same helper is used in the submenu so marks stay consistent.
2. A separator is inserted once, between the last pinned entry and the first
   unpinned one. It is omitted if there are no pinned entries, or none
   unpinned.
3. `Pin / unpin` is shown only when the history is non-empty. Its submenu
   lists every entry from the main menu, pinned ones marked with `★`.
   Activating one calls `db.toggle_pin(id)` and does not paste or touch the
   clipboard. The menu closes as usual; reopening shows the new order.
4. Digit mnemonics (1-9) still apply to the main list in display order, so
   pinned entries get the lowest numbers. They match the produced character,
   so on AZERTY the top row needs Shift (Shift+2 selects entry 2).
   Keypad digits 1-9 also select entries by position (Num Lock on). They are
   handled in a `key-press-event` handler on the menu that pops the menu down
   and activates the item; out-of-range digits are ignored and other keys
   (arrows, Enter, Escape, mnemonics) pass through unchanged.
5. `Clear history` keeps pinned entries (`clear()` only deletes unpinned).

Tests: `db.rs` covers the reported `pinned` flag, pins surviving a re-copy,
and the limit applying to unpinned entries only. The submenu itself is
checked manually: pin, unpin, and verify order in the database and the
screenshot of the open menu.

## Phase 4: robustness (done, except the two checks only the user can do)

Status legend: [x] done, [ ] to do.

- [x] **Menu popup while shortcut keys are held.** Cinnamon runs the command
  while `Super+V` is still down, and its key grab makes GTK's menu grab fail
  (`no trigger event for menu popup`): no menu, and a leaked helper window.
  `menu.rs` retries `popup_at_widget` every 25 ms (up to 2 s) until the grab
  succeeds, and ends the interaction only after the menu was actually mapped.
- [x] **Quiet logs.** The retry loop made GTK print one warning per attempt.
  GLib 2.80 sends these through the structured log writer (per-domain
  handlers are bypassed), so `main.rs` installs a writer function that drops
  exactly `no trigger event for menu popup` and forwards everything else to
  the default writer. A normal run now prints nothing.
- [x] **Overlapping `show` calls.** A held or repeated shortcut re-ran `show`
  while a menu was still retrying its grab, leaking about 13 helper windows per
  press. `State::menu_active` now makes later calls a no-op until the first
  interaction ends.
- [x] **Clean shutdown.** SIGTERM and SIGINT quit the application through the
  main loop (`glib::unix_signal_add_local`); `main` then runs a
  `wal_checkpoint(TRUNCATE)`. Verified: exit code 0, WAL file empty, all rows
  present after reopening.
- [x] **Oversized text.** (The 100 000-byte limit tested here is superseded by
  the Phase 6 storage budget.) Verified (99 999 B stored, 100 000 B stored, 100 001 B ignored, 20 MB ignored with RSS +11 MB and the daemon still responsive). Original checklist: text just under `MAX_BYTES` (100 000) is
  stored, just over is ignored, a multi-megabyte copy does not freeze the
  daemon or grow memory noticeably, and a normal copy still works afterwards.
  Also binary-ish text and labels containing `<`, `&` and `_`.
- [x] **Real-app paste matrix** (RustRover left to the user, see table). Check paste with the real shortcut in:
  Firefox (text field and URL bar), RustRover, an Electron app, gnome-terminal
  (done), and a plain GTK entry (done). Tune `SETTLE` in `paste.rs` if any of
  them pastes late, the wrong thing, or nothing. Record results in the
  table at the end of this section.
- [x] **Shortcut modifiers still held (paste side).** Verified with a real `Super+V` key press in Chrome and xed: pasted text, no `Super+Ctrl+V` side effect. Original note: Confirm
  `--clearmodifiers` is enough with the real `Super+V` (should not turn the
  paste into `Super+Ctrl+V`). If not, poll until the keys are released.
- [x] **Clipboard persistence.** Verified. Copy in app A, close A, open the menu: the
  entry is still there (SQLite) and pastes (we re-own the clipboard).
- [x] **Self-change loop.** Verified: no duplicate rows after a paste. Pasting does not create duplicate rows.
- [x] **Password-manager copies.** The daemon skips copies that carry the
  `x-kde-passwordManagerHint` target. Confirmed working by the user with
  their password manager. Bug found and fixed while testing with a fake
  source: when the source exits, the session's clipboard service (`csd-clipboard`)
  re-offers the same text without the marker, and it was stored. The watcher
  now remembers the text of the last skipped copy (`State::last_secret`) and
  ignores it until a different copy arrives. This becomes a user setting in Phase 7 (default:
  skip).
- [ ] **Fresh login.** After a real logout and login: the daemon is running,
  `Super+V` works without rewriting `custom-list`, and the original Diodon is
  not running.
- [x] **Database safety.** Directory 0700, file 0600, WAL mode.

Not planned: a "don't record copies from app X" ignore list (the clipboard
does not tell us the source window reliably), and a search/filter UI.

Results table (fill in while testing):

| App | Paste | Notes |
|---|---|---|
| GTK entry | ok | |
| gnome-terminal | ok | Ctrl+Shift+V |
| Google Chrome (textarea) | ok | text with `é _ < &` |
| Google Chrome (URL bar) | ok | Ctrl+L then paste |
| xed | ok | saved file matches |
| Firefox | n/a | not installed |
| RustRover | to test by the user | not tested here: it is the IDE in use, pasting could alter open files |
| Electron app | not tested | Chrome (same engine) is ok; Discord is installed if wanted |

## Phase 5: integration

1. **Binary location.** `cargo install --path .` puts it in `~/.cargo/bin`.
   Use the absolute path in the shortcut and autostart entries, because
   desktop sessions may not have that directory on `PATH`.
2. **Autostart.** Write `~/.config/autostart/diodon-next.desktop`:
   ```
   [Desktop Entry]
   Type=Application
   Name=diodon-next
   Exec=/home/quentin/.cargo/bin/diodon-next daemon
   Terminal=false
   X-GNOME-Autostart-enabled=true
   ```
   The file is installed at `~/.config/autostart/diodon-next.desktop`. It takes
   effect at the next login; to start it earlier run
   `diodon-next daemon` in the background after stopping Diodon and CopyQ.
3. **Cinnamon keybinding.** Either use Settings → Keyboard → Shortcuts →
   Custom Shortcuts, or script it with `gsettings` on
   `org.cinnamon.desktop.keybindings` (`custom-list` plus a
   `custom-keybindings/<name>` entry with `name`, `command`, `binding`).
   The command is `/home/quentin/.cargo/bin/diodon-next show`. Suggested
   binding: `<Primary><Alt>v`. Check it does not clash with the existing
   Diodon shortcut.
4. **Retire Diodon.** Stop and disable the existing Diodon (`pkill diodon`,
   remove its autostart) so there are not two clipboard managers fighting
   for ownership.

## Phase 6: multi-format entries (rich text and images)

Entries stop being "a string". An entry holds a plain-text form, optional
`text/html` and `text/rtf` forms, or a PNG image. Work is split in steps that
each leave the app working; do them in order.

### Decisions (agreed)

- **Identity is a hash.** Two copies are the same entry when the SHA-256 of all
  their stored formats matches (`glib::Checksum`, no new crate). The same words
  copied plain and copied formatted are therefore two entries.
- **Rich is the default paste.** Enter, click, keypad digits and digit
  mnemonics paste the stored rich form. **Shift+Enter and Shift+click** paste
  the plain text instead. That "plain mode" exists only for those two
  gestures: it is deliberately ignored for every shortcut (keypad digits,
  mnemonics, anything else), and it is **not a setting**.
- **Markers, in this order before the label:** 📌 pinned, 📸 image, 🤖 rich
  text. They combine (`📌🤖 text`). They replace the old star.
- **Storage budget, no per-entry limit.** History is limited by a total size
  budget (default 200 MB, a guess to adjust) plus the entry count. When the
  budget is exceeded, the oldest unpinned entries are removed silently. No
  notification: removal is expected behaviour. The old 100 000-byte text limit
  goes away.
- **Rich text toggle** in settings (default on); images toggle too.

### 6a. Schema v2, hash identity, storage budget (text only, no visible change except the markers)

```sql
PRAGMA user_version = 2;
CREATE TABLE entries (
  id         INTEGER PRIMARY KEY,
  hash       TEXT    NOT NULL UNIQUE,  -- SHA-256 hex over all stored formats
  kind       TEXT    NOT NULL DEFAULT 'text',  -- 'text' | 'image'
  text       TEXT,                     -- plain text (label source); NULL for images
  html       BLOB,                     -- text/html bytes, as offered
  rtf        BLOB,                     -- text/rtf bytes, as offered
  image      BLOB,                     -- PNG bytes
  thumb      BLOB,                     -- small PNG for the menu
  width      INTEGER,
  height     INTEGER,
  size       INTEGER NOT NULL,         -- total bytes of all stored formats
  created_at INTEGER NOT NULL,
  used_at    INTEGER NOT NULL,         -- strictly increasing, as today
  pinned     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX entries_used_at ON entries(used_at DESC);
```

- **Hash input** is unambiguous: for each present format in a fixed order
  (text, html, rtf, image), feed a one-byte tag, the 8-byte length, then the
  bytes.
- **Migration v1 to v2** rebuilds the table inside one transaction (SQLite
  cannot drop the old `UNIQUE(content)` in place): copy the file to
  `history.db.v1.bak` first, create the new table, copy rows computing each
  hash in Rust (SQLite has no SHA-256), keep `pinned` and `used_at`, drop the
  old table, set `user_version = 2`. Also set
  `PRAGMA auto_vacuum = INCREMENTAL` and run one `VACUUM`, because a SQLite
  file never shrinks otherwise.
- **Budget enforcement** after every insert, in one transaction: first the
  entry-count limit, then, while `SUM(size)` of unpinned rows exceeds the
  budget, delete the oldest unpinned row (by `used_at`). Then
  `PRAGMA incremental_vacuum`. Rules that follow from "no per-entry limit":
  - the entry just stored is **never** evicted by its own insert, so a single
    entry bigger than the whole budget is kept and everything older unpinned
    goes;
  - pinned entries are never evicted and do not count toward the budget (the
    user decides how much to pin).
- **Light listing.** `recent()` must not load blobs. It returns `id`, `kind`,
  `pinned`, a label source (first part of `text`), flags (`has_html`,
  `has_rtf`), `width`, `height` and `thumb`. A new `get(id)` loads the full
  entry only when one is picked.
- **Self-change check** (`last_set`) compares hashes instead of text. If
  testing shows that reading our own clipboard write back gives a different
  hash (encoding or newline changes), fall back to ignoring the next
  owner-change event(s) within a short window after we set the clipboard.
- **Password hint** check is unchanged and still runs before anything is read
  or stored; `last_secret` compares plain text.
- **Tests:** in-memory SQLite for migration (v1 file in, v2 out, pins and order
  kept), hash determinism and ambiguity cases, dedupe, budget eviction (oldest
  first, pinned kept, oversized newest kept), count limit, light `recent()`.

### 6b. Rich text

- **Capture.** After the password-hint check, read plain text and, if the
  rich setting is on, the whitelisted formats `text/html` and `text/rtf` via
  `wait_for_contents`/`request_contents`. Ignore every other target (for
  example `chromium/x-source-url`). Bytes are stored exactly as offered.
- **What becomes an entry**, in order: plain text present, so a text entry
  (with html/rtf attached when offered); else an image target present, so an
  image entry (6c); else only HTML or RTF, so a rich entry whose label is the
  fixed text `(formatted text)` plus its size. No tag stripping: we never
  parse HTML.
- **Paste.** Rich (default): offer `text/html`, `text/rtf` and plain text
  together. Plain (Shift+Enter, Shift+click only): `set_text`, as today. The
  bindings do not wrap `gtk_clipboard_set_with_data`, so this needs one
  `unsafe` call, isolated in a small module (`clip_out.rs`) through
  `gtk::ffi`. The callback hands out the right bytes per requested target;
  the clear callback drops the boxed data. It only copies byte buffers.
- **Security rules** (keep them): never render or parse the HTML (no webview,
  no sanitiser crate); store only the whitelisted formats; the menu always
  shows the plain text, so what is visible is never HTML; the 🤖 marker is the
  cue that Enter pastes more than what the label shows, and Shift+Enter is the
  way to paste only the visible text. HTML can carry link targets, hidden
  elements and tracking attributes, so it makes the plaintext database more
  sensitive; the rich toggle lets the user turn that off.
- **Tests:** round trip each format through the isolated module (offer, then
  read back from a second clipboard client), plain vs rich target lists,
  HTML-only copy, and the unsafe module run under `cargo test` with a real
  display.

### 6c. Images

- **Capture** only when no plain text is offered. Read `image/png` raw bytes
  with `wait_for_contents` (no decode and re-encode of large screenshots);
  fall back to `wait_for_image` and encode PNG with `gdk_pixbuf` when the
  source offers another format. Hash the PNG bytes.
- **Off the main thread.** Thumbnail creation (scale to about 48 px high, PNG)
  and any re-encoding run on a worker (`gio::spawn_blocking`, passing only
  `Vec<u8>`; `PixbufLoader` objects are created and used inside the worker).
  The main thread then does the insert. This keeps the daemon answering other
  applications' clipboard requests while a big image is processed.
- **Menu.** An image row shows the thumbnail and `📸 Image 1920x1080`, built as
  a `gtk::MenuItem` containing a box with an image and an accel label (so
  mnemonics and keypad digits still work).
- **Paste.** Decode the PNG into a pixbuf and `Clipboard::set_image`. Shift
  has no effect on images (there is no alternate form).
- **Tests:** PNG round trip, same image copied twice gives one row, eviction
  by budget, manual paste into a browser, an image editor and a chat app.

### 6d. Shift+Enter and Shift+click (plain paste)

The menu cannot tell a mouse click from a keystroke in its `activate` signal,
so decide from the triggering event. One function, `wants_plain(event type,
keyval, state)`, used by every entry's activate handler:

- true only for a key press of `Return`, `KP_Enter` or `ISO_Enter` with Shift
  held, or a button release with Shift held;
- false for everything else, in particular key presses of digits and keypad
  digits (the AZERTY top row needs Shift to type a digit, so Shift being held
  there means nothing) and calls made by the keypad handler.

The decision comes from `gtk::current_event()` inside the handler, with the
keypad handler marking its own activations as non-plain. Unit-test the
function with the combinations above; verify manually with real Shift+Enter,
Shift+click and Shift+digit on this AZERTY keyboard.

### Open points for Phase 6

- Confirm in a prototype that `gtk::current_event()` is the key or button
  event during menu activation (6d).
- Default budget (200 MB) is a guess; revisit after real use. Pinned entries
  do not count toward it (decided in 6a), so a user who pins many large images
  can exceed it on purpose.
- README: update the usage section when this lands (it still describes the
  100 000-byte limit and the star).

## Phase 7: settings window in the main menu (no status icon)

A tray icon was considered and dropped: it would need `libxapp` and an FFI
shim, against the "few dependencies" goal, and the main menu is where the user
already is. The menu gets two new entries:

```
...entries...
-------------------
Pin / unpin  >
Pause recording        <- check item
Settings...
Clear history
```

1. **Settings storage.** A `settings` table in the existing SQLite database
   (key/value), loaded into `State` at startup. Defaults apply when a key is
   missing. No extra file format, no new crate.
2. **Settings list.**
   - Skip password-manager copies (default on).
   - Record rich text (default on).
   - Record images (default on).
   - Maximum number of entries (default 50).
   - Storage budget in MB (default 200).
   - A `Quit daemon` button.

   Deliberately **not** a setting: the plain paste behaviour (Shift+Enter /
   Shift+click), per Phase 6.
3. **Settings window.** A plain `gtk::Window` with check buttons and spin
   buttons. Changes apply immediately in the running daemon; lowering the entry
   count or the budget prunes right away. Opened from the menu item and by
   `diodon-next settings` (forwarded to the daemon like `show`).
4. **One-shot mode.** When `show` runs without a daemon, choosing `Settings...`
   must keep the process alive until the window closes (treat it like a paste
   in progress for `on_finish`), then exit.
5. **Pause recording.** A check item in the menu: while on, the watcher stores
   nothing (the clipboard itself keeps working). State persists in settings.
6. **Trade-off to document.** With no tray icon there is no visible sign that
   the daemon runs. The README covers `pgrep`, `diodon-next quit` and the
   Quit button.

## Phase 8: README and clean-install documentation (done: `README.md`)

Status: written and checked. Verified from scratch in an `ubuntu:24.04`
container: the apt package list plus `rustup` builds and installs the binary
with no missing libraries. Found there: Ubuntu 24.04's apt `rustc` is 1.75, too
old for the 2024 edition, so `rustup` is required. The `gsettings` shortcut
snippet was run verbatim against a scratch keyfile backend (empty list and
existing list), and the list-reload trick on the real settings. Not tested: a
full Mint VM from the ISO, and the GUI shortcut steps.

A `README.md` that takes a clean Linux Mint (Cinnamon, X11) install to a
working setup. Every command in it must be tested in a clean environment (a
fresh VM or container is ideal; at minimum on a throwaway user account).

Contents:

1. What it is, what it is not (personal tool, X11 only, no Wayland).
2. **System packages** (apt): `build-essential`, `pkg-config`, `libgtk-3-dev`,
   `xdotool`, `x11-utils` (for `xwininfo`) and `x11-xserver-utils` if used,
   and `curl` for rustup. List only what the build and runtime really need,
   found by testing on a clean install.
3. **Rust toolchain** via `rustup`, and how to load `~/.cargo/env`.
4. **Build and install**: `cargo install --path . --locked`; where the binary
   goes (`~/.cargo/bin`); how to confirm it is on `PATH`.
5. **Autostart**: the `.desktop` file, and a one-line command that writes it.
6. **Global shortcut** on Cinnamon: GUI steps (Settings, Keyboard, Shortcuts,
   Custom) and the equivalent `gsettings` commands, including the
   `custom-list` rewrite needed so Cinnamon notices the change.
7. **Replacing Diodon / CopyQ**: how to stop and disable them.
8. **Usage**: menu keys (arrows, Enter, digits with Shift on AZERTY, keypad
   digits, `P` for pin, `C` for clear), pinning, where the data lives and its
   permissions.
9. **Uninstall**: remove the binary, autostart file, shortcut and data.
10. **Troubleshooting**: shortcut does nothing, menu does not appear, paste
    goes to the wrong window, terminals needing Ctrl+Shift+V.

A small `install.sh` that performs steps 2 to 6 is optional; the README must
stand on its own without it.

## Phase 9: rename

The project will be renamed before any wider use. The name is **not chosen**
(candidates discussed: Balloon, Pelican, Puffer). A bare "Balloon" collides
with the Ubuntu `balloon` package (a Go password-hashing tool, same
`/usr/bin/balloon`) and with crates.io, PyPI and npm names, so use a distinct
binary and package name (for example `<name>-clipboard`) even if the display
name stays friendly. Check availability again for the final choice, and look
at trademark databases (not done).

Places to change:

- `Cargo.toml` package name, and the binary name.
- The application ID in `src/main.rs` (`dev.quentin.DiodonNext`) and the
  `usage:` text.
- The data directory `~/.local/share/diodon-next/`: migrate it to the new
  name on first start (move, do not orphan the user's history).
- `~/.config/autostart/diodon-next.desktop` and the Cinnamon shortcut command
  on this machine, plus `README.md` and this plan.
- The `DIODON_DEBUG` variable mentioned under Testing.

## Later / maybe

- File lists (`text/uri-list`).
- Optional encryption at rest.
- A `.deb` package (cargo-deb, built in an `ubuntu:24.04` container so the
  glibc floor is 2.39), an app icon and `.desktop` file, a `LICENSE`, and a
  first-run command that registers the Cinnamon shortcut. Requires Phase 7
  first.

## Testing strategy

- Unit tests for `db.rs` (in-memory SQLite) and for the label formatting
  function (flatten, truncate, escape).
- Manual checklist after each phase, using real apps: Firefox, a text editor,
  gnome-terminal, an Electron app.
- A debug log to stderr behind the `DIODON_DEBUG` environment variable.
- For Phase 6: migration tests from a real v1 database copy, and a manual
  matrix of source apps (browser, spreadsheet, office suite, terminal, image
  editor) for rich text and images.

## Known risks

| Risk | Mitigation |
|---|---|
| Menu does not grab the keyboard without a focused window | Phase 1 helper window prototype |
| Paste races (wrong window, nothing pasted) | `windowactivate --sync`, tuned delays |
| Some apps ignore synthetic `Ctrl+V` | Content stays on the clipboard for manual paste |
| `gtk-rs` 0.18 is maintenance-only | Fine for a personal tool, pin the version |
| First compile is slow | One-time cost, incremental builds are fast |
| Secrets stored in plaintext | File permissions, password-manager hint (HTML and images widen what is stored; toggles in Phase 7) |
| Rich paste inserts more than the menu shows | 🤖 marker, plain text always shown, Shift+Enter for plain, never render HTML |
| Huge copies stall the daemon | Image work on a worker thread; budget eviction in one transaction |
| `unsafe` clipboard FFI call | Isolated module, byte copying only, round-trip tests |
| Our own clipboard write read back with a different hash | Hash first, short ignore window as fallback |
| SQLite file never shrinks | `auto_vacuum = INCREMENTAL` plus `incremental_vacuum` after eviction |
