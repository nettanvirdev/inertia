//! Which shell a terminal tab starts, and how it is asked to report its folder.
//!
//! The same shell the rest of the app runs commands with - `pwsh` if the person
//! has it, Windows PowerShell otherwise, `$SHELL` everywhere else - because a
//! person whose `shell` tool answers in PowerShell 7 and whose terminal pane
//! answers in 5.1 has two different machines in one window.
//!
//! The arguments are NOT the ones the `shell` tool uses, and that is the whole
//! difference between the two. `shell` runs one command and wants nothing of
//! the person's setup in the way of it: `-NoProfile`, `-NonInteractive`. A
//! terminal tab is the opposite - it is the person's own shell, so it gets
//! their profile, their prompt, their aliases and their completions, which is
//! the entire reason for having a pty rather than a pipe.
//!
//! ## Getting a shell to say where it is, without the person watching
//!
//! A pty is a stream of bytes. It does not know what a working directory is, so
//! the shell is asked to volunteer one from its prompt as an escape sequence
//! the window never shows (see `ansi.rs` for the reading half).
//!
//! It used to be typed in. The setup script was written into the pty a moment
//! after the shell started, which works perfectly and is also visible: a shell
//! echoes what is typed at it, so every new tab opened with nine lines of
//! somebody else's PowerShell across the top, each continuation marked `>>`. It
//! looked like a bug in the app because it was one.
//!
//! PowerShell will run a script before it hands over the prompt, so it is
//! passed as `-EncodedCommand`. Encoded rather than quoted because the script
//! contains double quotes, backticks and `$(...)`, and every layer between here
//! and the shell - the pty, CreateProcess, PowerShell's own parser - has an
//! opinion about those. Base64 of UTF-16LE has no opinion. `-NoExit` keeps the
//! session interactive afterwards, and the profile still loads first, which
//! matters: the prompt being wrapped has to exist already.
//!
//! POSIX shells have no equivalent that does not mean replacing the file the
//! person's own configuration lives in, so those are still typed - but as one
//! line rather than nine, which is one echoed line and no continuations.
//!
//! ## Why this wraps the prompt instead of setting it
//!
//! The shell being decorated is the user's, with their theme and whatever
//! prompt framework they have installed. Replacing `prompt` would work
//! perfectly and would silently delete oh-my-posh, starship, or the thing they
//! spent an evening on. So the existing function is captured first and called
//! from inside the replacement, and the worst case is a prompt that renders
//! exactly as it did plus an invisible escape sequence.
//!
//! ## Why a failure here is not a failure
//!
//! Every line below is fire-and-forget. If the shell rejects it - an unusual
//! shell, a locked-down execution policy, a `set -u` that dislikes the
//! assignment - the terminal still works completely and the only thing lost is
//! the folder shown in the pane's header. Nothing may be written here that
//! could stop a shell starting.

/// What kind of shell this is, which is all the integration needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    PowerShell,
    Posix,
}

/// A shell to start, and how.
#[derive(Debug, Clone)]
pub struct Shell {
    pub kind: Kind,
    pub program: String,
    /// The shell's own arguments, plus anything the integration needed.
    pub args: Vec<String>,
    /// A line to type in once the shell is up, for shells that cannot be told
    /// at launch. `None` is a terminal that works and does not report its
    /// folder, which is not an error.
    pub typed: Option<String>,
}

const POWERSHELL_SCRIPT: &str = concat!(
    "if (-not $global:__inertiaPrompt) {\n",
    "  $global:__inertiaPrompt = $function:prompt\n",
    "  function global:prompt {\n",
    "    $out = & $global:__inertiaPrompt\n",
    "    $esc = [char]27; $bel = [char]7\n",
    "    [Console]::Write(\"$esc]9;9;`\"$($PWD.Path)`\"$bel\")\n",
    "    return $out\n",
    "  }\n",
    "}",
);

/// The POSIX side, as a single line.
///
/// `${PWD}` is not escaped: a path with a space in it is fine in a file URL,
/// and a path with a control character in it is not something a shell can hand
/// us anyway. `precmd_functions` in zsh, `PROMPT_COMMAND` in bash, and the
/// existing value is kept rather than replaced.
const POSIX_LINE: &str = concat!(
    r#"__inertia_cwd() { printf '\033]7;file://%s%s\007' "$HOSTNAME" "$PWD"; }; "#,
    r#"if [ -n "$ZSH_VERSION" ]; then precmd_functions+=(__inertia_cwd); "#,
    r#"else PROMPT_COMMAND="__inertia_cwd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; fi; "#,
    "__inertia_cwd\n",
);

/// What PowerShell's `-EncodedCommand` wants: base64 of UTF-16LE.
///
/// Not a convenience - it is the only encoding the switch accepts, and getting
/// it wrong produces a shell that starts and silently does nothing. Written out
/// here rather than pulled from a crate because it is sixteen lines and the
/// alphabet has not changed since 1987.
pub fn encode_powershell(script: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = ((triple >> (18 - i * 6)) & 0x3f) as usize;
                out.push(char::from(ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Is `pwsh` on the PATH.
///
/// The same question `inertia-tools`' shell tool asks, and for the same reason:
/// 5.1 has no `&&`, no ternary and no null-coalescing, so the two are different
/// shells wearing one name.
fn has_pwsh() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path).any(|dir| {
        extensions
            .iter()
            .any(|ext| dir.join(format!("pwsh{ext}")).is_file())
    })
}

/// The shell a terminal tab should start, with its folder reporting wired up.
pub fn shell_for_pty() -> Shell {
    if cfg!(windows) {
        let program = if has_pwsh() { "pwsh" } else { "powershell.exe" };
        return Shell {
            kind: Kind::PowerShell,
            program: program.to_string(),
            args: vec![
                "-NoLogo".to_string(),
                "-NoExit".to_string(),
                "-EncodedCommand".to_string(),
                encode_powershell(POWERSHELL_SCRIPT),
            ],
            typed: None,
        };
    }

    let program = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string());
    Shell {
        kind: Kind::Posix,
        program,
        // Interactive and a login shell, which is what makes their prompt,
        // their aliases and their completions work. This is the whole point of
        // a pty; without it the person has a worse shell than the one in their
        // own terminal app.
        args: vec!["-i".to_string(), "-l".to_string()],
        typed: Some(POSIX_LINE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoding_is_the_one_the_switch_accepts() {
        // UTF-16LE then base64: "hi" is 68 00 69 00.
        assert_eq!(encode_powershell("hi"), "aABpAA==");
        assert_eq!(encode_powershell("a"), "YQA=");
        assert_eq!(encode_powershell(""), "");
    }

    #[test]
    fn the_script_survives_the_round_trip_through_base64() {
        let encoded = encode_powershell(POWERSHELL_SCRIPT);
        assert!(!encoded.contains(' '));
        // The prompt is wrapped rather than replaced, which is what stops this
        // silently deleting somebody's starship setup.
        assert!(POWERSHELL_SCRIPT.contains("$global:__inertiaPrompt = $function:prompt"));
        assert!(POWERSHELL_SCRIPT.contains("& $global:__inertiaPrompt"));
    }

    #[test]
    fn a_terminal_tab_gets_the_persons_profile_unlike_the_shell_tool() {
        let shell = shell_for_pty();
        assert!(!shell.args.iter().any(|arg| arg == "-NoProfile"));
        if cfg!(windows) {
            assert_eq!(shell.kind, Kind::PowerShell);
            // -NoExit, or the shell runs the setup script and leaves.
            assert!(shell.args.iter().any(|arg| arg == "-NoExit"));
            assert!(shell.typed.is_none());
        } else {
            assert_eq!(shell.kind, Kind::Posix);
            assert!(shell.args.iter().any(|arg| arg == "-i"));
            // One line, because a shell echoes what is typed at it.
            assert_eq!(shell.typed.as_deref().map(|line| line.lines().count()), Some(1));
        }
    }
}
