# jayclip

Keep the things you copy close at hand. jayclip is a clipboard manager for Linux
that lets you quickly find and reuse recent text, formatted snippets, and images.

- 🔎 **Pick up where you left off:** open your history with a keyboard shortcut,
  then paste an item into the window you were using.
- 📌 **Keep useful items handy:** pin entries so they stay at the top of your
  history; browse thumbnails for copied images.
- 📝 **Preserve formatting:** HTML and RTF snippets keep their rich-text formatting,
  with an option to paste plain text instead.
- 🔒 **Keep history on your device:** entries are stored locally with restrictive
  file permissions. Passwords are not skipped by default; an optional setting
  can skip copies that carry a recognized password-manager hint.

jayclip is a native GTK 3 application. Its full automatic paste workflow uses
X11 tools, so it is intended for Linux desktop sessions running X11. GTK UI and
clipboard support may work in other GTK sessions, but Wayland-only sessions do
not support automatic target-window detection and paste.

## At a glance

Once installed and running, you:

1. Copy things normally.
2. Press your shortcut (for example `Super+V`) to open the history menu.
3. Press `1`-`9`, click, or use the arrow keys to paste an entry.

| Marker      | Meaning                                                     |
|-------------|-------------------------------------------------------------|
| 📌          | Pinned — stays at the top, never auto-removed               |
| 🖌️          | Has rich formatting (HTML/RTF); pastes formatted by default |
| (thumbnail) | An image entry                                              |

## Requirements

jayclip is a native GTK 3 desktop application. The source uses Unix-specific
APIs, and there is no declared or tested support matrix for distributions or
CPU architectures. Linux is the intended platform; the project has been tested
on Linux Mint 22.3 with Cinnamon/X11, and its build was also verified in a stock
Ubuntu 24.04 container.

For the full automatic paste workflow, use an **X11** session and install
`xdotool` and `xprop` (`x11-utils` on Debian/Ubuntu-based systems). GTK may run
under Wayland, but automatic target-window detection and paste are X11-specific
and are not supported in Wayland-only sessions.

To build from source, you need Rust 1.85 or newer, a C compiler, `pkg-config`,
and GTK 3 development libraries. SQLite is built from bundled sources. The
Debian/Ubuntu package commands below are examples; use the equivalent packages
for your distribution. Allow about 1.5 GB of free disk space for the Rust
toolchain and build, plus a few minutes for the first compile.

To check whether your current session is X11:

```sh
echo $XDG_SESSION_TYPE   # must print: x11
```

## Install

The steps below are a one-time setup example for Debian/Ubuntu-based systems
using Cinnamon. Other distributions need their equivalent development packages,
and other desktop environments can use their own autostart and global-shortcut
settings. Steps 1–5 get it running; step 6 is only needed if you already use
another clipboard manager.

### 1. Install the system packages

```sh
sudo apt update
sudo apt install build-essential pkg-config libgtk-3-dev xdotool x11-utils curl git
```

What each one is for:

| Package                         | Why it is needed                                                                                                 |
|---------------------------------|------------------------------------------------------------------------------------------------------------------|
| `build-essential`, `pkg-config` | C compiler and library lookup. SQLite is compiled from bundled sources, so `libsqlite3-dev` is **not** required. |
| `libgtk-3-dev`                  | GTK 3 headers for the build                                                                                      |
| `xdotool`                       | Refocuses the target window and sends Ctrl+V                                                                     |
| `x11-utils`                     | Provides `xprop`, used to detect terminals so paste uses Ctrl+Shift+V there                                      |
| `curl`, `git`                   | Fetch rustup and this repository                                                                                 |

### 2. Install the Rust toolchain

Use `rustup`. The Rust shipped in Ubuntu 24.04's apt repository (1.75) is too
old — this project uses the 2024 edition and needs Rust 1.85 or newer.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
rustc --version
```

New terminals pick up `~/.cargo/bin` automatically; the installer adds it to
`~/.profile` and `~/.bashrc`.

### 3. Build and install the program

```sh
git clone <this repository> jayclip
cd jayclip
cargo install --path . --locked
```

This places the binary at `~/.cargo/bin/jayclip`. The commands below use the
full path on purpose, because desktop sessions do not always have that directory
on `PATH`.

### 4. Start the background daemon at login

The daemon is what watches the clipboard and records your history. Set it to
start automatically:

```sh
mkdir -p ~/.config/autostart
cat > ~/.config/autostart/jayclip.desktop <<EOF
[Desktop Entry]
Type=Application
Name=jayclip
Comment=Clipboard manager
Exec=$HOME/.cargo/bin/jayclip daemon
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
```

To start it right now, without logging out and back in:

```sh
setsid ~/.cargo/bin/jayclip daemon >/dev/null 2>&1 &
```

> **Tip:** if new copies stop showing up, the daemon has probably stopped. Check
> with `pgrep -ax jayclip`; if it is gone, run the command above again.
> Copies are only recorded while the daemon is running.

### 5. Add the global shortcut

The shortcut simply runs `jayclip show` to open the menu. Set it up either
way:

**Using the GUI (easiest):** open *Menu → Preferences → Keyboard → Shortcuts →
Custom Shortcuts*, click *Add custom shortcut*, then fill in:

- **Name:** `jayclip`
- **Command:** `/home/<you>/.cargo/bin/jayclip show` (use your real home path)
- Click the binding column and press the keys you want, for example `Super+V`.

**Using the terminal (does the same thing):**

```sh
BIN="$HOME/.cargo/bin/jayclip"
KB=org.cinnamon.desktop.keybindings
CURRENT=$(gsettings get $KB custom-list)

# Pick the first unused id: custom0, custom1, ...
N=0; while echo "$CURRENT" | grep -q "'custom$N'"; do N=$((N+1)); done
ID=custom$N
K=$KB.custom-keybinding:/org/cinnamon/desktop/keybindings/custom-keybindings/$ID/

gsettings set $K name "jayclip"
gsettings set $K command "$BIN show"
gsettings set $K binding "['<Super>v']"

# Register the new id in the list.
if [ "$CURRENT" = "@as []" ]; then NEW="['$ID']"; else NEW=$(echo "$CURRENT" | sed "s/]$/, '$ID']/"); fi
gsettings set $KB custom-list "$NEW"
```

Prefer a different key? Replace `<Super>v` with another binding, such as
`<Primary><Alt>v`. If the combination is already taken, Cinnamon flags a
conflict in the Keyboard settings — free it there first.

If you later change an existing shortcut's command from the terminal and
Cinnamon does not notice, force it to reload the list:

```sh
L=org.cinnamon.desktop.keybindings
OLD=$(gsettings get $L custom-list)
gsettings set $L custom-list "['__dummy__']"; sleep 1; gsettings set $L custom-list "$OLD"
```

### 6. Replace another clipboard manager (optional)

Two clipboard managers running at once will fight over the clipboard. If you
previously used Diodon or CopyQ, stop them and keep them from starting again:

```sh
pkill -x diodon; pkill -x copyq

# Diodon autostarts from a system file; override it for your user only:
cp /etc/xdg/autostart/diodon-autostart.desktop ~/.config/autostart/
sed -i 's/^X-GNOME-Autostart-enabled=.*/X-GNOME-Autostart-enabled=false/' ~/.config/autostart/diodon-autostart.desktop
grep -q '^X-GNOME-Autostart-enabled' ~/.config/autostart/diodon-autostart.desktop \
  || echo 'X-GNOME-Autostart-enabled=false' >> ~/.config/autostart/diodon-autostart.desktop
```

Also remove or repoint Diodon's old keyboard shortcut (*Keyboard → Shortcuts →
Custom Shortcuts*), or it will keep relaunching Diodon.

## Usage

Copy things as usual. When you need something back, press your shortcut and the
history menu opens at the mouse pointer.

### Keys in the menu

| Key                           | Action                                                                                  |
|-------------------------------|-----------------------------------------------------------------------------------------|
| Up / Down, then Enter         | Move to an entry and paste it                                                           |
| `1`-`9` (top-row digits)      | Paste the entry at that position. On an AZERTY keyboard use Shift (Shift+2 for entry 2) |
| `1`-`9` (keypad, Num Lock on) | Paste the entry at that position                                                        |
| `P`                           | Open the *Pin / unpin* submenu                                                          |
| Escape                        | Close the menu without pasting                                                          |
| Shift+Enter or Shift+click    | Paste the **plain-text** version of an entry                                            |

### How pasting works

When you pick an entry, jayclip copies it to the clipboard, brings your
previous window back to the front, and sends the paste keystroke for you:
Ctrl+V normally, or Ctrl+Shift+V in terminals (gnome-terminal, kitty,
alacritty, xterm, tilix, konsole, and similar).

Rich-text entries (marked 🖌️) paste **with their formatting** by default. Use
Shift+Enter or Shift+click to paste the plain-text version instead. Note that
Shift is ignored for the digit and keypad shortcuts — those always paste the
default (formatted) version.

Only the normal clipboard is watched, not the "select to copy" primary
selection.

### Rich text and images

- **Rich text** entries offer HTML or RTF and are marked 🖌️. The menu shows a
  plain-text preview; pasting keeps the formatting. Supported formats are
  `text/html`, `text/rtf`, and `application/rtf` when the source provides them.
- **Images** are stored as PNG. The menu row shows a thumbnail (scaled to fit
  within 48×48 pixels, aspect ratio preserved) alongside the original
  dimensions; pasting pastes the image. If a copy contains both usable text and
  an image, the text is stored.

### Pinning, history size, and cleanup

- Entries are listed newest first. **Pinned** entries (📌) stay at the top and
  are never auto-removed or cleared.
- History keeps up to **100 unpinned entries** within a **200 MB** storage
  budget. There is no per-entry size limit; when either limit is reached, the
  oldest unpinned entries are dropped.
- **Settings → History size** controls how many recent entries the menu *shows*
  (5, 10, 20, 50, or 100; default 20). This only changes the displayed count —
  it never deletes anything. Pinned entries always appear above the recent list.
- **Settings → Clear history** removes unpinned entries and keeps pinned ones.
- **Settings → Pause recording** is unchecked while recording is active; check
  it to temporarily stop adding entries (the clipboard still works normally).
  The choice persists across daemon restarts.

### Passwords

**Do not store passwords** is off by default. When enabled, it skips a copy
only when the source advertises the `x-kde-passwordManagerHint` clipboard hint.

> **Caveat:** some password managers and desktop clipboard services do not
> preserve this X11 hint, so protection is best-effort. Keep your password
> manager's clipboard-clearing timer enabled. Turning the setting on shows this
> same explanation for confirmation.

### Commands

You normally never run these by hand — autostart and the shortcut do it — but
they are available:

```sh
jayclip daemon   # run the background process (autostart does this)
jayclip show     # open the menu (the shortcut does this)
jayclip quit     # stop the daemon (there is no Quit menu item)
```

If you run `show` while no daemon is running, the menu opens once and exits
afterwards, and nothing is recorded during that time.

## Where your data lives

Your history is a single SQLite database:

```
~/.local/share/jayclip/history.db
```

It runs in WAL mode, so you will also see `-wal` and `-shm` files while the
daemon is running. Because history can contain secrets, the directory is created
with mode `0700` and the file with `0600`.

To peek at what is stored (requires Python 3):

```sh
python3 - <<'PY'
import sqlite3, os
db = sqlite3.connect(os.path.expanduser('~/.local/share/jayclip/history.db'))
for pinned, text in db.execute('select pinned, substr(coalesce(text, \'\'), 1, 60) from entries order by pinned desc, used_at desc'):
    print(pinned, repr(text))
PY
```

## Update

```sh
cd jayclip && git pull
cargo install --path . --locked
~/.cargo/bin/jayclip quit; setsid ~/.cargo/bin/jayclip daemon >/dev/null 2>&1 &
```

## Uninstall

```sh
~/.cargo/bin/jayclip quit
cargo uninstall jayclip
rm ~/.config/autostart/jayclip.desktop
rm -r ~/.local/share/jayclip        # deletes your history
```

Then remove the `jayclip` entry under *Keyboard → Shortcuts → Custom
Shortcuts*.

## Troubleshooting

**The shortcut does nothing.**
Check the daemon is running (`pgrep -ax jayclip`) and that the menu opens
when run directly (`~/.cargo/bin/jayclip show`). If that works but the
shortcut does not, the shortcut's command is wrong or Cinnamon did not reload
it — re-apply it as in step 5. Another program may also own the same key
combination.

**The menu appears but nothing gets pasted.**
Make sure `xdotool` is installed (`command -v xdotool`). Some applications
ignore synthetic key presses (remote desktop clients, some games). The entry is
still on the clipboard, so you can paste it by hand.

**It pastes a stray `v`, or does something odd in a terminal.**
Your terminal's window class is not in the built-in list, so Ctrl+V was sent
instead of Ctrl+Shift+V. Find the class with `xprop WM_CLASS` (then click the
window) and add it to `TERMINALS` in `src/paste.rs`.

**The build fails with "The system library `atk` / `gdk-3.0` required ... was
not found".**
`libgtk-3-dev` is missing — install it (step 1).

**The build fails complaining about `edition` or an unknown feature.**
You are probably using the old apt Rust (1.75 on Ubuntu 24.04). Check with
`which cargo`: it should be `~/.cargo/bin/cargo`. Install via `rustup` (step 2)
and open a new terminal.

**Another clipboard manager keeps restoring old content.**
Stop it (step 6). Also check `pgrep -ax "diodon|copyq|clipit|parcellite|gpaste"`.

## Development

```sh
cargo build
cargo test
cargo clippy
```

Run a throwaway copy without touching your real history:

```sh
XDG_DATA_HOME=/tmp/jayclip-test ./target/debug/jayclip daemon
```
