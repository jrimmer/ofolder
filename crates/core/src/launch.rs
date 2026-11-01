//! Opening a folder in a file manager or a terminal.
//!
//! Commands are built as a program and a list of arguments, never as text for
//! a shell, so a folder name cannot be mistaken for a command. The file
//! manager is the one the desktop's configuration names for folders, unless
//! the settings say otherwise.

use std::path::{Path, PathBuf};

use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Splits a command line into words, honouring single and double quotes and a
/// backslash before the next character. There is no expansion of any kind.
pub fn split_command(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    in_word = true;
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(ch);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `Exec` line of a desktop entry, with field codes filled in: `%f` and
/// `%F` get the folder path, `%u` and `%U` its URL, other codes are dropped.
/// An entry with no folder code gets the folder as a last argument.
pub fn command_from_exec(exec: &str, folder: &Path) -> Option<Command> {
    let words = split_command(exec);
    if words.is_empty() {
        return None;
    }
    let mut had_code = false;
    let mut out: Vec<String> = Vec::new();
    for word in words {
        let mut piece = String::new();
        let mut chars = word.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                piece.push(c);
                continue;
            }
            match chars.next() {
                Some('f' | 'F') => {
                    had_code = true;
                    piece.push_str(&folder.to_string_lossy());
                }
                Some('u' | 'U') => {
                    had_code = true;
                    piece.push_str(&file_url(folder));
                }
                Some('%') => piece.push('%'),
                _ => {}
            }
        }
        if !piece.is_empty() {
            out.push(piece);
        }
    }
    if !had_code {
        out.push(folder.to_string_lossy().into_owned());
    }
    let program = out.first()?.clone();
    Some(Command {
        program,
        args: out[1..].to_vec(),
        cwd: None,
    })
}

/// The desktop entry id the user's configuration names as the default for folders.
fn default_folder_handler(mimeapps: &str) -> Option<String> {
    let mut in_defaults = false;
    for line in mimeapps.lines().map(str::trim) {
        if line.starts_with('[') {
            in_defaults = line == "[Default Applications]";
        } else if in_defaults && let Some(rest) = line.strip_prefix("inode/directory=") {
            return rest
                .split(';')
                .map(str::trim)
                .find(|s| !s.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

fn exec_line(desktop_entry: &str) -> Option<String> {
    let mut in_entry = false;
    for line in desktop_entry.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry && let Some(rest) = line.strip_prefix("Exec=") {
            return Some(rest.to_owned());
        }
    }
    None
}

/// How to open `folder` in a file manager. `mimeapps` is the text of the
/// user's `mimeapps.list`; `desktop_entry` reads a desktop entry by id.
pub fn file_manager_command(
    settings: &Settings,
    mimeapps: &str,
    desktop_entry: &dyn Fn(&str) -> Option<String>,
    folder: &Path,
) -> Result<Command, String> {
    if let Some(configured) = &settings.file_manager {
        let mut words = split_command(configured);
        if words.is_empty() {
            return Err("the file_manager setting is empty".to_owned());
        }
        let program = words.remove(0);
        words.push(folder.to_string_lossy().into_owned());
        return Ok(Command {
            program,
            args: words,
            cwd: None,
        });
    }
    let id = default_folder_handler(mimeapps)
        .ok_or("no file manager is set for folders: add one with the file_manager setting")?;
    let entry =
        desktop_entry(&id).ok_or_else(|| format!("the file manager {id} could not be found"))?;
    let exec = exec_line(&entry).ok_or_else(|| format!("{id} has no command to run"))?;
    command_from_exec(&exec, folder).ok_or_else(|| format!("{id} has an empty command"))
}

/// How to open a terminal in `folder`: the setting, else `$TERMINAL`, else
/// `xdg-terminal-exec` if it is installed. The terminal starts in the folder
/// because that is its working directory, so no terminal-specific flag is needed.
pub fn terminal_command(
    settings: &Settings,
    env_terminal: Option<&str>,
    in_path: &dyn Fn(&str) -> bool,
    folder: &Path,
) -> Result<Command, String> {
    let chosen = settings
        .terminal
        .clone()
        .or_else(|| {
            env_terminal
                .filter(|t| !t.trim().is_empty())
                .map(str::to_owned)
        })
        .or_else(|| in_path("xdg-terminal-exec").then(|| "xdg-terminal-exec".to_owned()))
        .ok_or("no terminal is set: add one with the terminal setting")?;
    let mut words = split_command(&chosen);
    if words.is_empty() {
        return Err("the terminal setting is empty".to_owned());
    }
    let program = words.remove(0);
    Ok(Command {
        program,
        args: words,
        cwd: Some(folder.to_path_buf()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> &'static Path {
        Path::new("/home/j/My Folder")
    }

    #[test]
    fn a_command_line_splits_on_spaces_and_keeps_quoted_words_whole() {
        assert_eq!(split_command("flea --gui"), ["flea", "--gui"]);
        assert_eq!(
            split_command("run 'two words' \"and more\" x\\ y"),
            ["run", "two words", "and more", "x y"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
        assert_eq!(split_command("a ''"), ["a", ""]);
    }

    #[test]
    fn nothing_in_a_command_line_is_expanded() {
        assert_eq!(
            split_command("echo $HOME `id` $(x)"),
            ["echo", "$HOME", "`id`", "$(x)"]
        );
    }

    #[test]
    fn a_folder_code_is_replaced_with_the_path_and_the_folder_name_with_spaces_stays_one_argument()
    {
        let c = command_from_exec("fm --new-window %F", folder()).expect("command");
        assert_eq!(c.program, "fm");
        assert_eq!(c.args, ["--new-window", "/home/j/My Folder"]);
    }

    #[test]
    fn a_url_code_gets_an_encoded_file_url() {
        let c = command_from_exec("fm %u", folder()).expect("command");
        assert_eq!(c.args, ["file:///home/j/My%20Folder"]);
    }

    #[test]
    fn other_field_codes_are_dropped_and_a_percent_is_kept() {
        let c = command_from_exec("fm %i %c 100%% %F", folder()).expect("command");
        assert_eq!(c.args, ["100%", "/home/j/My Folder"]);
    }

    #[test]
    fn an_entry_with_no_folder_code_gets_the_folder_as_its_last_argument() {
        let c = command_from_exec("fm --flag", folder()).expect("command");
        assert_eq!(c.args, ["--flag", "/home/j/My Folder"]);
    }

    #[test]
    fn an_empty_exec_line_gives_no_command() {
        assert_eq!(command_from_exec("", folder()), None);
        assert_eq!(command_from_exec("   ", folder()), None);
    }

    const MIMEAPPS: &str = "[Added Associations]\ninode/directory=other.desktop;\n\n[Default Applications]\ninode/directory=flea.desktop;nautilus.desktop;\ntext/plain=editor.desktop\n";

    fn entries(id: &str) -> Option<String> {
        (id == "flea.desktop").then(|| "[Desktop Entry]\nName=Flea\nExec=/usr/bin/flea --gui %F\n[Desktop Action x]\nExec=ignored".to_owned())
    }

    #[test]
    fn the_file_manager_is_the_one_the_desktop_names_for_folders() {
        let c = file_manager_command(&Settings::default(), MIMEAPPS, &entries, folder())
            .expect("command");
        assert_eq!(c.program, "/usr/bin/flea");
        assert_eq!(c.args, ["--gui", "/home/j/My Folder"]);
    }

    #[test]
    fn the_setting_overrides_the_desktops_choice() {
        let s = Settings {
            file_manager: Some("thunar --new".to_owned()),
            ..Settings::default()
        };
        let c = file_manager_command(&s, MIMEAPPS, &entries, folder()).expect("command");
        assert_eq!((c.program.as_str(), c.args.len()), ("thunar", 2));
        assert_eq!(c.args[1], "/home/j/My Folder");
    }

    #[test]
    fn with_no_default_or_a_missing_desktop_entry_the_reason_is_given_not_a_guess() {
        let none = file_manager_command(
            &Settings::default(),
            "[Default Applications]\n",
            &entries,
            folder(),
        );
        assert!(
            none.expect_err("no handler")
                .contains("file_manager setting")
        );
        let lost = file_manager_command(
            &Settings::default(),
            "[Default Applications]\ninode/directory=gone.desktop\n",
            &entries,
            folder(),
        );
        assert!(lost.expect_err("missing").contains("gone.desktop"));
    }

    #[test]
    fn an_association_outside_the_defaults_section_is_not_the_default() {
        let only_added = "[Added Associations]\ninode/directory=flea.desktop\n";
        assert!(
            file_manager_command(&Settings::default(), only_added, &entries, folder()).is_err()
        );
    }

    #[test]
    fn the_terminal_starts_in_the_folder_because_that_is_its_working_directory() {
        let s = Settings {
            terminal: Some("ghostty --class=x".to_owned()),
            ..Settings::default()
        };
        let c = terminal_command(&s, None, &|_| false, folder()).expect("command");
        assert_eq!(
            (c.program.as_str(), c.args.as_slice()),
            ("ghostty", ["--class=x".to_owned()].as_slice())
        );
        assert_eq!(c.cwd.as_deref(), Some(folder()));
    }

    #[test]
    fn the_terminal_falls_back_to_the_environment_then_to_the_desktops_launcher() {
        let none = Settings::default();
        assert_eq!(
            terminal_command(&none, Some("kitty"), &|_| true, folder())
                .expect("env")
                .program,
            "kitty"
        );
        assert_eq!(
            terminal_command(&none, None, &|p| p == "xdg-terminal-exec", folder())
                .expect("launcher")
                .program,
            "xdg-terminal-exec"
        );
        assert!(
            terminal_command(&none, Some("  "), &|_| false, folder())
                .expect_err("none")
                .contains("terminal setting")
        );
    }
}
