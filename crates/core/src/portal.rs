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
