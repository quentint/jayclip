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
- `diodon-next show`: bound to the global shortcut. Activates the running
  daemon's `show` action and exits. If no daemon is running, it starts one
  and shows the menu.

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

## Phase 4: robustness

- **Shortcut modifiers still held.** Verify `--clearmodifiers` is enough with
  a `Ctrl+Alt+V`-style shortcut. If it is not, add a small delay or poll
  `xdotool` / XQueryKeymap until the keys are released.
- **Timing.** Tune the pre-activation and pre-key delays by testing in a slow
  app (Electron, Firefox). Make them constants.
- **Clipboard persistence.** Test: copy in app A, close A, open the menu. The
  entry must still be there (it is in SQLite) and pasting it must work (we
  re-own the clipboard in `paste`).
- **Self-change loop.** Confirm that pasting does not create duplicate rows.
- **Database safety.** Confirm file modes (0700 directory, 0600 file) and that
  a crash does not corrupt the WAL database.
- **Sensitive data.** Skip hinted entries. Consider adding a "don't record
  when the source window class is X" list, for example a password manager.
- **Large or odd text.** Test with multi-megabyte text, binary-ish text and
  text with `<`, `&` and `_` in the label.
- **Daemon lifetime.** Handle SIGTERM and let the application quit cleanly.

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
   X-GNOME-Autostart-enabled=true
   ```
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

## Phase 6: later (not in v1)

- Search or filter. `Gtk.Menu` has none, so this would need a small popup
  window with an entry and a list.
- Images and file lists (`text/uri-list`): blob table, multi-target
  clipboard ownership, thumbnails in the menu.
- Configuration file (history size, terminal list, ignored apps).
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
