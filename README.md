# diodon-next

A small personal clipboard manager for Linux Mint (Cinnamon) on **X11**,
inspired by [Diodon](https://github.com/diodon-dev/diodon).

It keeps your clipboard history in a local SQLite database. A global shortcut
opens a native menu at the mouse pointer; picking an entry pastes it into the
window that had focus.

This is a personal tool, not a distributable product. It is written in Rust with
GTK 3, and it only works on X11. Wayland sessions are not supported.

Current scope: plain text, rich text (`text/html`, `text/rtf`, and
`application/rtf` when offered), and PNG clipboard images. Settings include
**Do not store passwords**, enabled by default. Rich-text entries are marked 🤖
and paste rich by default; Shift+Enter and Shift+click paste plain text.

## Requirements

- Linux Mint 22.x (Cinnamon), X11. Tested on Mint 22.3; the build was also
  verified from scratch on a stock Ubuntu 24.04 container
- A user account with `sudo` for the one-time package install
- About 1.5 GB of disk space for the Rust toolchain and the build, and a few
  minutes for the first compile

Check you are on X11:

```sh
echo $XDG_SESSION_TYPE   # must print: x11
```

## Install

### 1. System packages

```sh
sudo apt update
sudo apt install build-essential pkg-config libgtk-3-dev xdotool x11-utils curl git
```

| Package | Why |
|---|---|
| `build-essential`, `pkg-config` | C compiler and library lookup. SQLite is compiled from bundled sources, so no `libsqlite3-dev` is needed. |
| `libgtk-3-dev` | GTK 3 headers for the build |
| `xdotool` | Refocuses the target window and sends Ctrl+V |
| `x11-utils` | `xprop`, used to read the focused window's class (to paste with Ctrl+Shift+V in terminals) |
| `curl`, `git` | Fetch rustup and this repository |

### 2. Rust toolchain

Use `rustup`: the Rust in Ubuntu 24.04's apt repository (1.75) is too old for
this project, which uses the 2024 edition (Rust 1.85 or newer).

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
rustc --version
```

New terminals pick up `~/.cargo/bin` automatically; the installer adds it to
`~/.profile` and `~/.bashrc`.

### 3. Build and install the program

```sh
git clone <this repository> diodon-next
cd diodon-next
cargo install --path . --locked
```

This puts the binary at `~/.cargo/bin/diodon-next`. The commands below use the
absolute path, because desktop sessions do not always have that directory on
`PATH`.

### 4. Start the daemon at login

```sh
mkdir -p ~/.config/autostart
cat > ~/.config/autostart/diodon-next.desktop <<EOF
[Desktop Entry]
Type=Application
Name=diodon-next
Comment=Personal clipboard manager
Exec=$HOME/.cargo/bin/diodon-next daemon
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
```

To start it right now, without logging out:

```sh
setsid ~/.cargo/bin/diodon-next daemon >/dev/null 2>&1 &
```

If clipboard entries stop appearing, check `pgrep -ax diodon-next`. If the
`daemon` process is missing, start it again with the command above; new copies
are recorded only while it is running.

### 5. Add the global shortcut

The shortcut runs `diodon-next show`.

**With the GUI:** open *Menu → Preferences → Keyboard → Shortcuts → Custom
Shortcuts*, click *Add custom shortcut*, then:

- Name: `diodon-next`
- Command: `/home/<you>/.cargo/bin/diodon-next show` (use your real home path)
- Click the binding column and press your shortcut, for example `Super+V`.

**From a terminal** (this sets the same thing):

```sh
BIN="$HOME/.cargo/bin/diodon-next"
KB=org.cinnamon.desktop.keybindings
CURRENT=$(gsettings get $KB custom-list)

# Pick the first unused id: custom0, custom1, ...
N=0; while echo "$CURRENT" | grep -q "'custom$N'"; do N=$((N+1)); done
ID=custom$N
K=$KB.custom-keybinding:/org/cinnamon/desktop/keybindings/custom-keybindings/$ID/

gsettings set $K name "diodon-next"
gsettings set $K command "$BIN show"
gsettings set $K binding "['<Super>v']"

# Register the new id in the list.
if [ "$CURRENT" = "@as []" ]; then NEW="['$ID']"; else NEW=$(echo "$CURRENT" | sed "s/]$/, '$ID']/"); fi
gsettings set $KB custom-list "$NEW"
```

Replace `<Super>v` with another binding if you prefer, for example
`<Primary><Alt>v`. If the shortcut is already used by something else, Cinnamon
shows a conflict in the Keyboard settings; free it there first.

If you edit an existing shortcut's command from the command line and Cinnamon
does not pick it up, re-write the list to make it reload:

```sh
L=org.cinnamon.desktop.keybindings
OLD=$(gsettings get $L custom-list)
gsettings set $L custom-list "['__dummy__']"; sleep 1; gsettings set $L custom-list "$OLD"
```

### 6. Replace another clipboard manager (optional)

Two clipboard managers running at once fight over the clipboard. If you used
Diodon or CopyQ, stop them and keep them from starting:

```sh
pkill -x diodon; pkill -x copyq

# Diodon autostarts from a system file; override it for your user only:
cp /etc/xdg/autostart/diodon-autostart.desktop ~/.config/autostart/
sed -i 's/^X-GNOME-Autostart-enabled=.*/X-GNOME-Autostart-enabled=false/' ~/.config/autostart/diodon-autostart.desktop
grep -q '^X-GNOME-Autostart-enabled' ~/.config/autostart/diodon-autostart.desktop \
  || echo 'X-GNOME-Autostart-enabled=false' >> ~/.config/autostart/diodon-autostart.desktop
```

Also remove or repoint Diodon's old keyboard shortcut (Keyboard → Shortcuts →
Custom Shortcuts), or it will keep launching Diodon.

## Usage

Copy things as usual. Press your shortcut: a menu opens at the pointer.

| Key | Action |
|---|---|
| Up / Down, Enter | Move and paste the selected entry |
| Keypad `1`-`9` | Paste the entry at that position (Num Lock on) |
| Top-row digits `1`-`9` | Paste the entry at that position. On an AZERTY keyboard this needs Shift (Shift+2 selects entry 2) |
| `P` | Open the *Pin / unpin* submenu |
| `C` | Clear history (pinned entries are kept) |
| Escape | Close the menu |

Details:

- Entries offering HTML or RTF are marked 🤖. The menu shows their plain-text
  label; selecting one pastes rich content by default. Rich formats supported
  are `text/html`, `text/rtf`, and `application/rtf` when offered.
- Image-only clipboard entries are stored as PNG. Their menu row includes a
  thumbnail scaled to fit within 48×48 pixels (aspect ratio preserved) and the
  original dimensions; selecting one pastes the image. If a clipboard offers
  usable text and an image together, text is stored.
- Shift+Enter and Shift+click paste the plain-text form for entries that have
  one. Shift on top-row digit mnemonics or keypad shortcuts is ignored: those
  always paste the rich/default form.
- Entries are listed newest first. Pinned entries (marked 📌) stay at the top
  and are never removed or cleared.
- The history retains up to 100 unpinned entries, within a fixed 200 MB storage
  budget. There is no per-entry size limit; when either hard limit is reached,
  the oldest unpinned entries are removed.
- Settings → History size selects how many recent entries the menu displays
  (5, 10, 20, 50 or 100; default 20). Changing this only changes the displayed count; it
  does not delete entries. Pinned entries remain visible above the recent list.
- Picking an entry puts it on the clipboard, brings the previously focused
  window back to the front and sends Ctrl+V (Ctrl+Shift+V in terminals such as
  gnome-terminal, kitty, alacritty, xterm, tilix, konsole).
- **Do not store passwords** is enabled by default. It skips copies only when
  the source offers `x-kde-passwordManagerHint`. Some password managers or
  desktop clipboard services do not preserve this X11 hint, so this is best
  effort; keep the password manager's clipboard-clearing timer enabled. Turning
  the setting on shows this explanation for confirmation.
- Only the regular clipboard is watched, not the "select to copy" primary
  selection.

Commands:

```sh
diodon-next daemon   # run the background process (autostart does this)
diodon-next show     # open the menu (the shortcut does this)
diodon-next quit     # stop the daemon
```

If `show` is run while no daemon is running, it starts the menu once and exits
afterwards; nothing is recorded in that case.

## Where the data lives

`~/.local/share/diodon-next/history.db` (SQLite, in WAL mode, so you will also
see `-wal` and `-shm` files while the daemon runs). The directory is created
with mode `0700` and the file `0600`, since history may contain secrets. To
inspect it (Python 3 ships with Linux Mint):

```sh
python3 - <<'PY'
import sqlite3, os
db = sqlite3.connect(os.path.expanduser('~/.local/share/diodon-next/history.db'))
for pinned, text in db.execute('select pinned, substr(coalesce(text, \'\'), 1, 60) from entries order by pinned desc, used_at desc'):
    print(pinned, repr(text))
PY
```

## Update

```sh
cd diodon-next && git pull
cargo install --path . --locked
~/.cargo/bin/diodon-next quit; setsid ~/.cargo/bin/diodon-next daemon >/dev/null 2>&1 &
```

## Uninstall

```sh
~/.cargo/bin/diodon-next quit
cargo uninstall diodon-next
rm ~/.config/autostart/diodon-next.desktop
rm -r ~/.local/share/diodon-next        # deletes your history
```

Then remove the `diodon-next` entry under *Keyboard → Shortcuts → Custom
Shortcuts*.

## Troubleshooting

**The shortcut does nothing.**
Check the daemon is running (`pgrep -ax diodon-next`) and that the menu opens
from a terminal (`~/.cargo/bin/diodon-next show`). If that works but the
shortcut does not, the shortcut's command is wrong or Cinnamon did not reload
it; re-write the list as shown in step 5. Another program may also own the
same key combination.

**The menu appears but nothing is pasted.**
Make sure `xdotool` is installed (`command -v xdotool`). Some applications
ignore synthetic key presses (remote desktop clients, some games). The entry
is still on the clipboard, so paste it by hand.

**It pastes `v` or does something odd in a terminal.**
The terminal's window class is not in the built-in list, so Ctrl+V was sent
instead of Ctrl+Shift+V. Check the class with `xprop WM_CLASS` (click the
window) and add it to `TERMINALS` in `src/paste.rs`.

**The build fails with "The system library `atk` / `gdk-3.0` required ... was
not found".**
`libgtk-3-dev` is missing; install it (step 1).

**The build fails complaining about the `edition` or an unknown feature.**
You are probably using the old apt Rust (1.75 on Ubuntu 24.04). Check with
`which cargo`: it should be `~/.cargo/bin/cargo`. Install via `rustup`
(step 2) and open a new terminal.

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
XDG_DATA_HOME=/tmp/diodon-test ./target/debug/diodon-next daemon
```

`PLAN.md` has the design, the roadmap and the testing notes.
