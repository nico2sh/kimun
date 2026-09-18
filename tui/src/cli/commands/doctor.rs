//! `kimun doctor` — what this terminal can and cannot deliver.
//!
//! Terminals disagree about keys in ways no amount of code can paper over: a
//! `Ctrl` chord is a single byte outside the kitty keyboard protocol, and
//! several of those bytes belong to real keys (see
//! [`crate::keys::reachability`]). The result is a class of bug that looks like
//! kimün ignoring a keypress, with nothing in the app to point at — the report
//! that started this was "Backspace moves focus to Search", which is `0x08`
//! being both Backspace and `Ctrl+H`.
//!
//! So: one command that prints the terminal's answers and every binding's fate,
//! for a user to read or to paste into an issue. Text only and deliberately
//! so — it is meant to be pasted, not parsed.

use color_eyre::eyre::Result;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::app::ctrl_h::{self, CtrlHPolicy};
use crate::cli::helpers::load_settings;
use crate::keys::reachability::{Reach, TerminalKeys, reach, unreachable_actions};
use crate::settings::CtrlHSetting;

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let settings = load_settings(config_path)?;

    // Both terminal questions are conversations with a tty: the capability
    // query writes an escape sequence and waits for a reply, and the erase
    // character comes from the line discipline. Under a pipe — `kimun doctor >
    // out.txt`, or a shell substitution — there is nobody to answer, and
    // asking anyway produces a confident "not supported" for a terminal that
    // supports it fine. A diagnostic that lies when redirected is worse than
    // one that declines, so this asks only when someone can answer.
    let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let enhanced =
        is_tty && ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    let erase = if is_tty {
        ctrl_h::tty_erase_char()
    } else {
        None
    };
    // `resolve_with`, not `resolve`: the latter probes termios itself, which
    // would read the erase character behind the "not read — output is
    // redirected" line printed below and let the table contradict its own
    // disclaimer.
    let policy = CtrlHPolicy::resolve_with(settings.ctrl_h, enhanced, erase == Some(0x08));
    let keys = TerminalKeys::detected(enhanced, policy == CtrlHPolicy::Backspace);

    println!("kimün {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Terminal");
    println!("  TERM                 {}", env_or_unset("TERM"));
    println!("  TERM_PROGRAM         {}", env_or_unset("TERM_PROGRAM"));
    if is_tty {
        println!(
            "  kitty keyboard       {}",
            if keys.enhanced && !enhanced {
                "not needed — the Windows console reports keys, not bytes"
            } else if enhanced {
                "supported — keys arrive unambiguously"
            } else {
                "no reply — Ctrl chords share bytes with Tab, Enter and Esc"
            }
        );
        println!("  tty erase character  {}", describe_erase(erase));
        println!(
            "  ctrl_h = {:<12} {}",
            format!("{:?}", settings.ctrl_h).to_lowercase(),
            describe_policy(settings.ctrl_h, policy, enhanced, erase)
        );
    } else {
        println!("  kitty keyboard       not asked — output is redirected");
        println!("  tty erase character  not read — output is redirected");
        println!();
        println!("  Run `kimun doctor` directly in the terminal you use kimün in");
        println!("  to get these two answers. Until then the table below assumes");
        if keys.enhanced {
            // `enhanced` (the escape-sequence probe) is false here — nobody
            // answered — but `keys.enhanced` still folds in `cfg!(windows)`,
            // so this is the Windows console: it reports keys, not bytes, and
            // the table below reflects that rather than the worst case.
            println!("  Windows: the console reports keys, not bytes, so no");
            println!("  chord collides.");
        } else {
            println!("  the worst case: no protocol, so Ctrl chords collide.");
        }
    }

    println!();
    println!("Key bindings");
    let map = settings.key_bindings.to_hashmap();
    let mut rows: Vec<(String, Vec<String>)> = map
        .into_iter()
        .map(|(action, combos)| {
            let fates = combos
                .iter()
                .map(|c| match reach(*c, keys) {
                    Reach::Ok => c.to_string(),
                    Reach::Shadowed(by) => format!("{c} (arrives as {by})"),
                    Reach::Untransmitted => format!("{c} (not sent by this terminal)"),
                })
                .collect();
            (action.to_string(), fates)
        })
        .collect();
    rows.sort();
    for (action, fates) in &rows {
        println!("  {action:<22} {}", fates.join(" · "));
    }

    println!();
    let stranded = unreachable_actions(&settings.key_bindings, keys);
    if stranded.is_empty() {
        println!("Every bound action has a key this terminal can send.");
    } else {
        for u in &stranded {
            println!("! {} has no key this terminal can send.", u.action);
        }
        println!();
        println!("{}", stranded_advice(settings.ctrl_h, policy));
    }
    Ok(())
}

fn env_or_unset(var: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| "(unset)".to_string())
}

/// `^?` is the modern convention and `^H` the older one; naming both spellings
/// is the point, since this is the line a bug reporter is asked to read.
fn describe_erase(erase: Option<u8>) -> String {
    match erase {
        Some(0x7f) => "^? (0x7f) — the usual setting".to_string(),
        Some(0x08) => "^H (0x08) — same byte as Ctrl+H".to_string(),
        Some(c) => format!("0x{c:02x}"),
        None => "could not be read".to_string(),
    }
}

fn describe_policy(
    setting: CtrlHSetting,
    policy: CtrlHPolicy,
    enhanced: bool,
    erase: Option<u8>,
) -> String {
    match policy {
        CtrlHPolicy::Backspace => "0x08 is delivered as Backspace; the Ctrl+H chord is unreachable",
        CtrlHPolicy::Chord if enhanced => "Ctrl+H is a chord (the protocol keeps the keys apart)",
        CtrlHPolicy::Chord if setting == CtrlHSetting::Auto && erase == Some(0x08) => {
            // Unreachable in practice (`auto` would have chosen Backspace);
            // stated rather than left to a catch-all so a future change to
            // `resolve` cannot make this line quietly wrong.
            "Ctrl+H is a chord, though the erase character suggests otherwise"
        }
        CtrlHPolicy::Chord if setting == CtrlHSetting::Auto => {
            "Ctrl+H is a chord (the tty's erase character is not 0x08)"
        }
        CtrlHPolicy::Chord => "Ctrl+H is a chord",
    }
    .to_string()
}

/// What to do about a stranded action. Keyed on the *policy*, not the
/// setting: `auto` reaches Backspace by reading the tty, and the knob to turn
/// is `ctrl_h` either way — advice that only mentioned it under an explicit
/// `backspace` would send the `auto` user off to rebind a default.
fn stranded_advice(setting: CtrlHSetting, policy: CtrlHPolicy) -> &'static str {
    match (policy, setting) {
        (CtrlHPolicy::Backspace, CtrlHSetting::Backspace) => {
            "ctrl_h = \"backspace\" gives up the Ctrl+H chord, so any action\n\
             bound only to it is listed above. Rebind those you use."
        }
        (CtrlHPolicy::Backspace, _) => {
            "ctrl_h = \"auto\" chose Backspace because the tty's erase character\n\
             is ^H, so the Ctrl+H chord is unreachable and any action bound only\n\
             to it is listed above. Set ctrl_h = \"chord\" to keep the chord (a\n\
             ^H Backspace key then cannot delete), or rebind those actions."
        }
        (CtrlHPolicy::Chord, _) => {
            "Rebind those actions, or use a terminal that speaks the kitty\n\
             keyboard protocol (Kitty, Ghostty, foot, WezTerm with\n\
             enable_kitty_keyboard = true)."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The erase line is what a bug reporter is asked to read back, so both
    /// spellings have to be named — `0x08` is the whole reason this command
    /// exists.
    #[test]
    fn the_erase_line_names_both_conventions() {
        assert!(describe_erase(Some(0x7f)).contains("^?"));
        let bs = describe_erase(Some(0x08));
        assert!(bs.contains("^H"), "{bs}");
        assert!(bs.contains("Ctrl+H"), "{bs}");
        assert_eq!(describe_erase(Some(0x15)), "0x15");
        assert!(describe_erase(None).contains("could not be read"));
    }

    /// Each policy explains itself by its actual cause, so the line cannot
    /// claim the erase character decided something the protocol did.
    #[test]
    fn the_policy_line_gives_the_real_reason() {
        let backspace = describe_policy(
            CtrlHSetting::Auto,
            CtrlHPolicy::Backspace,
            false,
            Some(0x08),
        );
        assert!(backspace.contains("unreachable"), "{backspace}");

        let protocol = describe_policy(CtrlHSetting::Auto, CtrlHPolicy::Chord, true, Some(0x7f));
        assert!(protocol.contains("protocol"), "{protocol}");

        let erase = describe_policy(CtrlHSetting::Auto, CtrlHPolicy::Chord, false, Some(0x7f));
        assert!(erase.contains("erase character is not 0x08"), "{erase}");

        // An explicit `chord` is obeyed whatever the terminal says, so the
        // line must not offer the terminal as the reason.
        let explicit = describe_policy(CtrlHSetting::Chord, CtrlHPolicy::Chord, false, Some(0x08));
        assert_eq!(explicit, "Ctrl+H is a chord");
    }

    /// The advice names the setting that caused the stranding whenever the
    /// policy is Backspace — under `auto` as much as under an explicit
    /// `backspace` — because the knob is the same either way.
    #[test]
    fn the_advice_names_ctrl_h_under_auto() {
        let auto = stranded_advice(CtrlHSetting::Auto, CtrlHPolicy::Backspace);
        assert!(auto.contains("ctrl_h = \"auto\""), "{auto}");
        assert!(auto.contains("ctrl_h = \"chord\""), "{auto}");

        let explicit = stranded_advice(CtrlHSetting::Backspace, CtrlHPolicy::Backspace);
        assert!(explicit.contains("ctrl_h = \"backspace\""), "{explicit}");

        let chord = stranded_advice(CtrlHSetting::Auto, CtrlHPolicy::Chord);
        assert!(chord.contains("kitty"), "{chord}");
    }
}
