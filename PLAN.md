# diodon-next: implementation plan

A personal clipboard manager for X11 (Cinnamon). Text only. Stores history in
SQLite, shows it in a native GTK menu opened by a global shortcut, and pastes
the chosen entry into the window that had focus.

Not meant to be distributed. Optimise for "works on this machine" and few
dependencies.

## Decisions

| Topic | Choice | Reason |
|---|---|---|
| Language | Rust (2021 edition) | User preference |
| UI / clipboard | `gtk` 0.18 (GTK3 bindings) | Native menu, clipboard `owner-change` signal, themed like Cinnamon |
| Storage | SQLite via `rusqlite` with the `bundled` feature | Compiles SQLite from C (gcc is already installed), so no `libsqlite3-dev` is needed |
| Paste / focus | `xdotool` (subprocess) | User preference, simplest |
| Global shortcut | Cinnamon custom keybinding running `diodon-next show` | No code, no X key grabbing |
| Process model | Single-instance `gtk::Application` (D-Bus activation) | The `show` invocation activates the daemon, so no custom IPC |
| Content | Text only | Scope for v1 |
| Platform | X11 only | Current session is X11; Wayland would need a different design |

Dependencies in `Cargo.toml`: `gtk`, `rusqlite` (features: `bundled`). Nothing
else. Timestamps use `std::time`. Argument parsing uses `std::env::args`.

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

1. `menu.rs` labels pinned entries with a `★ ` prefix (`entry_label`). The
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

## Phase 4: robustness (current)

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
- [x] **Oversized text.** Verified (99 999 B stored, 100 000 B stored, 100 001 B ignored, 20 MB ignored with RSS +11 MB and the daemon still responsive). Original checklist: text just under `MAX_BYTES` (100 000) is
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

## Phase 6: images

Text-only today; the original Diodon had images on. Needed:

1. **Schema (migration to `user_version = 2`).** Add `kind TEXT NOT NULL
   DEFAULT 'text'` and `data BLOB` to `entries`. For images, `content` holds a
   stable key (hex hash of the PNG bytes) so the existing `UNIQUE(content)`
   dedupe keeps working, and `data` holds the PNG.
2. **Capture.** In the watcher, when no text is offered but an image target
   is (`image/png` via `Clipboard::request_image` / `wait_for_image`), encode
   to PNG with `gdk_pixbuf` (already part of GTK, no new crate), hash it, and
   upsert. Cap the stored size (setting, default a few MB) and ignore larger
   images.
3. **Menu.** Show a small thumbnail (scaled pixbuf, about 48 px high) with a
   label like `Image 1920x1080`, using a `gtk::ImageMenuItem`-style layout
   (box with image and label). Keypad and mnemonic shortcuts apply as for text.
4. **Paste.** `paste.rs` sets the clipboard with `set_image` (the pixbuf
   decoded from the blob) before sending the paste keystroke.
5. **Pruning and size.** Images count toward the same entry cap; add a total
   size cap for image blobs so the database cannot grow without bound.
6. **Tests.** Round trip: PNG in, same hash out, dedupe, prune; manual paste
   into an image-capable app (browser, image editor, chat).

## Phase 7: settings UI and status icon

Needs a small dedicated UI, not just a config file.

1. **Settings storage.** A `settings` table in the existing SQLite database
   (key/value), read at startup and on change. No extra file format and no
   new crate. Defaults apply when a key is missing.
2. **Settings (initial list).**
   - Skip password-manager copies (default: on).
   - History size (default: 50; replaces the constant `MAX_ENTRIES`).
   - Maximum text size (default 100 000 bytes).
   - Record images, and the maximum image size (Phase 6).
3. **Settings window.** A `gtk::Window` with checkboxes and spin buttons,
   opened from the status icon menu and by `diodon-next settings`. Changes
   apply immediately in the running daemon.
4. **Status icon.** A tray icon for the daemon, with a menu: Show history,
   Settings, Pause recording, Quit. Cinnamon supports the StatusNotifier /
   XApp protocol, so the plan is to use the XApp status icon through
   `libxapp` (already installed on Linux Mint) via its GObject introspection
   bindings or a small FFI shim. The `gtk::StatusIcon` API is deprecated, so
   it is the fallback only. Decide when starting this phase after checking what
   the Rust side can call without a new dependency.
5. **Pause recording.** A toggle that stops the watcher from storing entries
   (clipboard stays usable); shown in the tray icon state.

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

## Later / maybe

- File lists (`text/uri-list`).
- Optional encryption at rest.

## Testing strategy

- Unit tests for `db.rs` (in-memory SQLite) and for the label formatting
  function (flatten, truncate, escape).
- Manual checklist after each phase, using real apps: Firefox, a text editor,
  gnome-terminal, an Electron app.
- A debug log to stderr behind the `DIODON_DEBUG` environment variable.

## Known risks

| Risk | Mitigation |
|---|---|
| Menu does not grab the keyboard without a focused window | Phase 1 helper window prototype |
| Paste races (wrong window, nothing pasted) | `windowactivate --sync`, tuned delays |
| Some apps ignore synthetic `Ctrl+V` | Content stays on the clipboard for manual paste |
| `gtk-rs` 0.18 is maintenance-only | Fine for a personal tool, pin the version |
| First compile is slow | One-time cost, incremental builds are fast |
| Secrets stored in plaintext | File permissions, password-manager hint, ignore list |
