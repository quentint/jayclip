use std::process::Command;
use std::time::Duration;

/// Window class fragments (lowercase) of terminals that paste with Ctrl+Shift+V.
const TERMINALS: &[&str] = &[
    "terminal", "xterm", "kitty", "alacritty", "tilix", "konsole", "urxvt", "wezterm", "foot",
    "terminator", "st-256color",
];

/// Delay between the menu closing and refocusing the target window.
const SETTLE: Duration = Duration::from_millis(60);

pub struct Target {
    pub window: String,
    pub class: String,
}

fn xdotool(args: &[&str]) -> Option<String> {
    let out = Command::new("xdotool").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Capture the focused window. Must run before any of our UI appears.
pub fn active_target() -> Option<Target> {
    let window = xdotool(&["getactivewindow"])?;
    let class = xdotool(&["getwindowclassname", &window]).unwrap_or_default();
    Some(Target { window, class })
}

pub fn paste_combo(class: &str) -> &'static str {
    let c = class.to_lowercase();
    if TERMINALS.iter().any(|t| c.contains(t)) {
        "ctrl+shift+v"
    } else {
        "ctrl+v"
    }
}

/// Refocus the target and send the paste keystroke, off the GTK thread so the
/// main loop can keep serving clipboard requests.
pub fn send_paste(target: Option<Target>) {
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        let combo = match &target {
            Some(t) => {
                if xdotool(&["windowactivate", "--sync", &t.window]).is_none() {
                    eprintln!("diodon-next: could not refocus window {}", t.window);
                }
                paste_combo(&t.class)
            }
            None => "ctrl+v",
        };
        if xdotool(&["key", "--clearmodifiers", combo]).is_none() {
            eprintln!("diodon-next: xdotool key failed; content is on the clipboard");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_use_shift() {
        assert_eq!(paste_combo("Gnome-terminal"), "ctrl+shift+v");
        assert_eq!(paste_combo("Alacritty"), "ctrl+shift+v");
    }

    #[test]
    fn others_use_plain_ctrl_v() {
        assert_eq!(paste_combo("firefox"), "ctrl+v");
        assert_eq!(paste_combo("jetbrains-rustrover"), "ctrl+v");
        assert_eq!(paste_combo(""), "ctrl+v");
    }
}
