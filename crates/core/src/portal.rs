//! Finding which file-chooser backends are installed.
//!
//! The portal frontend reads `*.portal` files to learn which backends exist.
//! ofolder reads the same files, so a backend that is installed later is
//! found without a code change; whether it can be decorated depends on
//! whether an adapter is known for it.

use std::fs;
use std::path::{Path, PathBuf};

/// The D-Bus interface a backend implements to be a file chooser.
pub const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalFile {
    pub source: PathBuf,
    pub dbus_name: String,
    pub interfaces: Vec<String>,
}

impl PortalFile {
    pub fn provides_file_chooser(&self) -> bool {
        self.interfaces.iter().any(|i| i == FILE_CHOOSER)
    }

    /// The adapter id for this backend, if ofolder has one.
    pub fn adapter_id(&self) -> Option<&'static str> {
        adapter_id(&self.dbus_name)
    }
}

/// Reads one `.portal` file. The format is a small INI file with a `[portal]`
/// section; anything without a `DBusName` is not a backend.
pub fn parse_portal(source: &Path, text: &str) -> Option<PortalFile> {
    let mut in_portal = false;
    let mut dbus_name = None;
    let mut interfaces = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_portal = section == "portal";
        } else if in_portal && let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "DBusName" => dbus_name = Some(value.trim().to_owned()),
                "Interfaces" => {
                    interfaces = value
                        .split(';')
                        .map(str::trim)
                        .filter(|i| !i.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                _ => {}
            }
        }
    }
    Some(PortalFile {
        source: source.to_path_buf(),
        dbus_name: dbus_name?,
        interfaces,
    })
}

/// Reads every `*.portal` file in `dirs`. Unreadable folders and files are
/// skipped: a broken neighbour must not hide a working backend.
pub fn discover(dirs: &[PathBuf]) -> Vec<PortalFile> {
    let mut found = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "portal"))
            .collect();
        paths.sort();
        for path in paths {
            if let Ok(text) = fs::read_to_string(&path)
                && let Some(file) = parse_portal(&path, &text)
            {
                found.push(file);
            }
        }
    }
    found
}

/// The file-chooser preference list of a portal configuration file, in order,
/// or `None` when the file does not set one.
pub fn file_chooser_chain(portals_conf: &str) -> Option<Vec<String>> {
    let mut in_preferred = false;
    for line in portals_conf.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_preferred = section == "preferred";
        } else if in_preferred
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == FILE_CHOOSER
        {
            return Some(
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                    .map(str::to_owned)
                    .collect(),
            );
        }
    }
    None
}

/// A portal configuration with `own` first in the file-chooser chain, changing nothing else:
/// other lines, comments and the order of the other backends stay. `None` when it is already
/// first, so there is nothing to write. With no chain line one is added with `others` after it,
/// and with no `[preferred]` section that is added too.
pub fn put_first(portals_conf: &str, own: &str, others: &[&str]) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_preferred = false;
    let mut done = false;
    let mut had_preferred = false;
    for line in portals_conf.lines() {
        let trimmed = line.trim();
        if let Some(section) = trimmed.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if in_preferred && !done {
                out.push(chain_line(own, others));
                done = true;
            }
            in_preferred = section == "preferred";
            had_preferred |= in_preferred;
            out.push(line.to_owned());
        } else if in_preferred
            && !done
            && let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == FILE_CHOOSER
        {
            let rest: Vec<&str> = value
                .split(';')
                .map(str::trim)
                .filter(|e| !e.is_empty() && *e != own)
                .collect();
            if value.split(';').map(str::trim).next() == Some(own) {
                return None;
            }
            let mut chain = vec![own];
            chain.extend(rest);
            out.push(format!("{FILE_CHOOSER}={}", chain.join(";")));
            done = true;
        } else {
            out.push(line.to_owned());
        }
    }
    if !done {
        if !had_preferred {
            out.push("[preferred]".to_owned());
        }
        out.push(chain_line(own, others));
    }
    Some(out.join("\n") + "\n")
}

fn chain_line(own: &str, others: &[&str]) -> String {
    let mut chain = vec![own];
    chain.extend(others);
    format!("{FILE_CHOOSER}={}", chain.join(";"))
}

/// The backend ofolder should forward to: the first entry after `own` in the
/// file-chooser preference list of a portal configuration file that has an
/// adapter. A `*` entry cannot be resolved here and is skipped.
pub fn backend_after(portals_conf: &str, own: &str) -> Option<&'static str> {
    backend_after_installed(portals_conf, own, &|_| true)
}

/// Like [`backend_after`], but skips a backend that is named and not installed,
/// so a chain that still names an uninstalled one does not leave the dialog
/// forwarded to nothing.
pub fn backend_after_installed(
    portals_conf: &str,
    own: &str,
    installed: &dyn Fn(&str) -> bool,
) -> Option<&'static str> {
    let mut in_preferred = false;
    for line in portals_conf.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_preferred = section == "preferred";
        } else if in_preferred
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == FILE_CHOOSER
        {
            let entries: Vec<&str> = value
                .split(';')
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .collect();
            let at = entries.iter().position(|e| *e == own)?;
            return entries[at + 1..]
                .iter()
                .filter_map(|e| match *e {
                    "flea" => Some("flea"),
                    "gtk" => Some("gtk"),
                    "strata" => Some("strata"),
                    _ => None,
                })
                .find(|id| installed(id));
        }
    }
    None
}

/// The D-Bus name of the backend an adapter drives: the inverse of
/// [`adapter_id`].
pub fn dbus_name_for(adapter_id: &str) -> Option<&'static str> {
    match adapter_id {
        "flea" => Some("org.freedesktop.impl.portal.desktop.flea"),
        "gtk" => Some("org.freedesktop.impl.portal.desktop.gtk"),
        "strata" => Some("org.freedesktop.impl.portal.desktop.strata"),
        _ => None,
    }
}

/// The adapter ids ofolder knows, keyed by the backend's D-Bus name.
pub fn adapter_id(dbus_name: &str) -> Option<&'static str> {
    match dbus_name {
        "org.freedesktop.impl.portal.desktop.flea" => Some("flea"),
        "org.freedesktop.impl.portal.desktop.gtk" => Some("gtk"),
        "org.freedesktop.impl.portal.desktop.strata" => Some("strata"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn putting_ofolder_first_changes_only_the_chooser_line() {
        let conf = "[preferred]\ndefault=hyprland;gtk\n# a note\norg.freedesktop.impl.portal.FileChooser=flea;gtk\norg.freedesktop.impl.portal.Secret=gnome-keyring\n";
        let fixed = put_first(conf, "ofolder", &["gtk"]).unwrap();
        assert_eq!(
            fixed,
            "[preferred]\ndefault=hyprland;gtk\n# a note\norg.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\norg.freedesktop.impl.portal.Secret=gnome-keyring\n"
        );
    }

    #[test]
    fn ofolder_in_the_middle_is_moved_to_the_front_without_a_duplicate() {
        let conf = "[preferred]\norg.freedesktop.impl.portal.FileChooser=flea;ofolder;gtk\n";
        assert_eq!(
            put_first(conf, "ofolder", &[]).unwrap(),
            "[preferred]\norg.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\n"
        );
    }

    #[test]
    fn nothing_is_written_when_ofolder_already_leads() {
        let conf = "[preferred]\norg.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\n";
        assert_eq!(put_first(conf, "ofolder", &[]), None);
    }

    #[test]
    fn a_missing_chain_line_or_section_is_added() {
        assert_eq!(
            put_first("[preferred]\ndefault=gtk\n", "ofolder", &["gtk"]).unwrap(),
            "[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser=ofolder;gtk\n"
        );
        assert_eq!(
            put_first("", "ofolder", &["flea", "gtk"]).unwrap(),
            "[preferred]\norg.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\n"
        );
    }

    #[test]
    fn a_chain_line_in_a_comment_or_another_section_is_not_the_chain() {
        let conf = "# org.freedesktop.impl.portal.FileChooser=ofolder;gtk\n[other]\norg.freedesktop.impl.portal.FileChooser=gtk\n";
        let fixed = put_first(conf, "ofolder", &["gtk"]).unwrap();
        assert!(fixed.contains("[other]\norg.freedesktop.impl.portal.FileChooser=gtk\n"));
        assert!(
            fixed.ends_with("[preferred]\norg.freedesktop.impl.portal.FileChooser=ofolder;gtk\n")
        );
    }

    #[test]
    fn a_backend_that_is_named_but_not_installed_is_skipped_to_the_next_installed_one() {
        let conf = "[preferred]\norg.freedesktop.impl.portal.FileChooser=ofolder;strata;flea;gtk\n";
        assert_eq!(
            backend_after_installed(conf, "ofolder", &|id| id != "strata"),
            Some("flea")
        );
        assert_eq!(
            backend_after_installed(conf, "ofolder", &|id| id == "gtk"),
            Some("gtk")
        );
        assert_eq!(backend_after_installed(conf, "ofolder", &|_| false), None);
        assert_eq!(
            backend_after_installed(conf, "ofolder", &|_| true),
            Some("strata")
        );
    }

    #[test]
    fn the_file_chooser_chain_is_read_in_order() {
        let conf = "[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\n";
        assert_eq!(
            file_chooser_chain(conf),
            Some(vec!["ofolder".into(), "flea".into(), "gtk".into()])
        );
    }

    #[test]
    fn a_comment_that_names_the_chooser_is_not_the_chain() {
        let conf = "[preferred]\n# flea replaced: org.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\norg.freedesktop.impl.portal.FileChooser=flea;gtk\n";
        assert_eq!(
            file_chooser_chain(conf),
            Some(vec!["flea".into(), "gtk".into()])
        );
    }

    #[test]
    fn no_chooser_line_means_no_chain() {
        assert_eq!(file_chooser_chain("[preferred]\ndefault=gtk\n"), None);
        assert_eq!(
            file_chooser_chain("org.freedesktop.impl.portal.FileChooser=ofolder\n"),
            None,
            "outside [preferred] the line means nothing"
        );
    }

    use super::*;

    // Verbatim from /usr/share/xdg-desktop-portal/portals on 2026-11-01.
    const FLEA: &str = "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.flea\nInterfaces=org.freedesktop.impl.portal.FileChooser;\n";
    const GTK: &str = "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.gtk\nInterfaces=org.freedesktop.impl.portal.FileChooser;org.freedesktop.impl.portal.AppChooser;org.freedesktop.impl.portal.Print;org.freedesktop.impl.portal.Notification;org.freedesktop.impl.portal.Inhibit;org.freedesktop.impl.portal.Access;org.freedesktop.impl.portal.Account;org.freedesktop.impl.portal.Email;org.freedesktop.impl.portal.DynamicLauncher;org.freedesktop.impl.portal.Lockdown;org.freedesktop.impl.portal.Settings;\nUseIn=gnome\n";
    const KWALLET: &str = "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.kwallet\nInterfaces=org.freedesktop.impl.portal.Secret;\n";

    fn parse(text: &str) -> Option<PortalFile> {
        parse_portal(Path::new("x.portal"), text)
    }

    #[test]
    fn the_flea_portal_file_is_a_file_chooser_with_a_known_adapter() {
        let f = parse(FLEA).expect("parses");
        assert!(f.provides_file_chooser());
        assert_eq!(f.adapter_id(), Some("flea"));
    }

    #[test]
    fn the_gtk_portal_file_finds_the_file_chooser_among_many_interfaces() {
        let f = parse(GTK).expect("parses");
        assert_eq!(f.interfaces.len(), 11);
        assert!(f.provides_file_chooser());
        assert_eq!(f.adapter_id(), Some("gtk"));
    }

    #[test]
    fn a_secret_service_backend_is_not_a_file_chooser() {
        let f = parse(KWALLET).expect("parses");
        assert!(!f.provides_file_chooser());
    }

    #[test]
    fn a_file_chooser_without_an_adapter_is_found_but_not_decorated() {
        let text = "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.kde\nInterfaces=org.freedesktop.impl.portal.FileChooser;\n";
        let f = parse(text).expect("parses");
        assert!(f.provides_file_chooser());
        assert_eq!(f.adapter_id(), None);
    }

    #[test]
    fn every_adapter_id_round_trips_through_its_dbus_name() {
        for id in ["flea", "gtk", "strata"] {
            let name = dbus_name_for(id).expect("known adapter");
            assert_eq!(adapter_id(name), Some(id));
        }
        assert_eq!(dbus_name_for("kde"), None);
    }

    const CONF: &str = "[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk\n";

    #[test]
    fn the_backend_to_forward_to_is_the_next_one_after_ofolder_in_the_preference_list() {
        assert_eq!(backend_after(CONF, "ofolder"), Some("flea"));
    }

    #[test]
    fn a_wildcard_or_an_unknown_backend_after_ofolder_is_skipped_to_the_next_known_one() {
        let conf = "[preferred]\norg.freedesktop.impl.portal.FileChooser=ofolder;*;kde;strata\n";
        assert_eq!(backend_after(conf, "ofolder"), Some("strata"));
    }

    #[test]
    fn when_ofolder_is_not_in_the_list_or_nothing_follows_it_there_is_no_backend() {
        assert_eq!(
            backend_after(
                "[preferred]\norg.freedesktop.impl.portal.FileChooser=flea;gtk\n",
                "ofolder"
            ),
            None
        );
        assert_eq!(
            backend_after(
                "[preferred]\norg.freedesktop.impl.portal.FileChooser=gtk;ofolder\n",
                "ofolder"
            ),
            None
        );
        assert_eq!(backend_after("", "ofolder"), None);
    }

    #[test]
    fn a_file_chooser_line_outside_the_preferred_section_does_not_count() {
        let conf = "[other]\norg.freedesktop.impl.portal.FileChooser=ofolder;flea\n";
        assert_eq!(backend_after(conf, "ofolder"), None);
    }

    #[test]
    fn the_packaged_portal_file_is_a_file_chooser_backend_with_the_name_the_proxy_answers_on() {
        let f = parse_portal(
            Path::new("ofolder.portal"),
            include_str!("../../../packaging/ofolder.portal"),
        )
        .expect("parses");
        assert!(f.provides_file_chooser());
        assert_eq!(f.dbus_name, "org.freedesktop.impl.portal.desktop.ofolder");
    }

    #[test]
    fn a_file_without_a_dbus_name_is_not_a_backend() {
        assert_eq!(parse("[portal]\nInterfaces=a;\n"), None);
    }

    #[test]
    fn keys_outside_the_portal_section_are_ignored() {
        let text = "[other]\nDBusName=wrong\n[portal]\nDBusName=right\n";
        assert_eq!(parse(text).expect("parses").dbus_name, "right");
    }

    #[test]
    fn discovery_reads_portal_files_sorted_and_skips_everything_else() {
        let dir = std::env::temp_dir().join(format!("ofolder-portal-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(dir.join("b.portal"), GTK).expect("write");
        fs::write(dir.join("a.portal"), FLEA).expect("write");
        fs::write(dir.join("notes.txt"), FLEA).expect("write");
        fs::write(dir.join("broken.portal"), "not a portal file").expect("write");
        let found = discover(&[dir.clone(), dir.join("missing")]);
        let names: Vec<_> = found.iter().map(|f| f.dbus_name.as_str()).collect();
        assert_eq!(
            names,
            [
                "org.freedesktop.impl.portal.desktop.flea",
                "org.freedesktop.impl.portal.desktop.gtk"
            ]
        );
        fs::remove_dir_all(&dir).expect("cleanup");
    }
}
