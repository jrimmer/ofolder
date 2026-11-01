//! Which application asked for a dialog.
//!
//! The portal tells the backend the application's id, but only for sandboxed
//! apps. Measured on this machine (xdg-desktop-portal 1.22.1): an ordinary
//! app, even one started in a systemd scope named after its desktop entry,
//! arrives with an empty id. So there are three cases, and they are never
//! blurred together:
//!
//! - the portal gave an id: that is the identity;
//! - it did not, but a window had focus when the request arrived: its window
//!   class is a fair guess, kept in its own namespace as `class:<name>` so it
//!   can never be mistaken for a real id, and used only where a wrong guess is
//!   cheap or where the user wrote the `class:` key themselves;
//! - neither: the app is unknown, and per-app features simply do not apply.

use std::path::{Path, PathBuf};

use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The id the portal passed along.
    PortalId,
    /// The class of the window that had focus when the request arrived.
    FocusedWindow,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The portal id, `class:<name>`, or empty when unknown.
    pub key: String,
    pub source: Source,
}

impl Identity {
    pub fn unknown() -> Self {
        Self {
            key: String::new(),
            source: Source::Unknown,
        }
    }

    pub fn is_known(&self) -> bool {
        self.source != Source::Unknown
    }
}

/// Window classes that are never an application asking for a dialog: ofolder's
/// own panels and the dialogs themselves (a dialog already on screen may well
/// have focus when a second request arrives).
const NOT_AN_ASKER: [&str; 5] = [
    "ofolder.strip.",
    "com.thisisgm.flea.picker",
    "xdg-desktop-portal-gtk",
    "io.github.lgse.Strata.FileChooser",
    "",
];

pub fn resolve(portal_app_id: &str, focused_class: Option<&str>) -> Identity {
    let id = portal_app_id.trim();
    if !id.is_empty() {
        return Identity {
            key: id.to_owned(),
            source: Source::PortalId,
        };
    }
    match focused_class.map(str::trim) {
        Some(class)
            if !NOT_AN_ASKER
                .iter()
                .any(|n| class.starts_with(n) && (!n.is_empty() || class.is_empty())) =>
        {
            Identity {
                key: format!("class:{class}"),
                source: Source::FocusedWindow,
            }
        }
        _ => Identity::unknown(),
    }
}

/// What to do for a dialog before it opens.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub show_strip: bool,
    /// A folder to start the dialog in, only when the app named none.
    pub seed_folder: Option<PathBuf>,
}

/// While paused nothing is shown or moved. A start folder is used only when
/// the application supplied none (the user's answer to question 8, which is
/// also what keeps a document's own folder), the identity is known, and the
/// settings hold a rule under exactly that key.
pub fn plan(
    settings: &Settings,
    identity: &Identity,
    supplied_folder: Option<&Path>,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Plan {
    if settings.paused {
        return Plan {
            show_strip: false,
            seed_folder: None,
        };
    }
    let excluded = identity.is_known() && settings.excluded_apps.iter().any(|a| a == &identity.key);
    let seed_folder = if supplied_folder.is_some() || !identity.is_known() {
        None
    } else {
        settings
            .start_folders
            .get(&identity.key)
            .filter(|f| is_dir(f))
            .cloned()
    };
    Plan {
        show_strip: !excluded,
        seed_folder,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn any_dir(_: &Path) -> bool {
        true
    }

    #[test]
    fn a_portal_id_is_the_identity() {
        let i = resolve("org.mozilla.firefox", Some("firefox"));
        assert_eq!(
            (i.key.as_str(), i.source),
            ("org.mozilla.firefox", Source::PortalId)
        );
    }

    #[test]
    fn an_empty_id_falls_back_to_the_focused_window_in_its_own_namespace() {
        let i = resolve("", Some("zen"));
        assert_eq!(
            (i.key.as_str(), i.source),
            ("class:zen", Source::FocusedWindow)
        );
    }

    #[test]
    fn with_no_id_and_no_focused_window_the_app_is_unknown() {
        assert_eq!(resolve("", None), Identity::unknown());
        assert_eq!(resolve("  ", Some("")), Identity::unknown());
    }

    #[test]
    fn the_panels_and_other_dialogs_are_never_taken_for_the_app_that_asked() {
        for class in [
            "ofolder.strip.left",
            "ofolder.strip.right",
            "com.thisisgm.flea.picker",
            "xdg-desktop-portal-gtk",
            "io.github.lgse.Strata.FileChooser",
        ] {
            assert!(!resolve("", Some(class)).is_known(), "{class}");
        }
    }

    #[test]
    fn a_class_that_merely_starts_like_a_dialog_class_is_still_an_app() {
        assert!(resolve("", Some("ofolder-notes")).is_known());
    }

    #[test]
    fn a_class_identity_never_equals_a_real_portal_id() {
        let by_class = resolve("", Some("org.mozilla.firefox"));
        let by_id = resolve("org.mozilla.firefox", None);
        assert_ne!(by_class.key, by_id.key);
    }

    fn settings_with_rules() -> Settings {
        let mut start = BTreeMap::new();
        start.insert("class:zen".to_owned(), PathBuf::from("/home/j/Downloads"));
        start.insert(
            "org.example.Editor".to_owned(),
            PathBuf::from("/home/j/Work"),
        );
        Settings {
            start_folders: start,
            excluded_apps: vec!["class:kiosk".to_owned()],
            ..Settings::default()
        }
    }

    #[test]
    fn a_start_folder_rule_applies_when_the_app_named_no_folder() {
        let p = plan(
            &settings_with_rules(),
            &resolve("", Some("zen")),
            None,
            &any_dir,
        );
        assert_eq!(p.seed_folder, Some(PathBuf::from("/home/j/Downloads")));
        let p = plan(
            &settings_with_rules(),
            &resolve("org.example.Editor", None),
            None,
            &any_dir,
        );
        assert_eq!(p.seed_folder, Some(PathBuf::from("/home/j/Work")));
    }

    #[test]
    fn an_app_that_supplied_its_own_folder_keeps_it() {
        let p = plan(
            &settings_with_rules(),
            &resolve("org.example.Editor", None),
            Some(Path::new("/doc/folder")),
            &any_dir,
        );
        assert_eq!(p.seed_folder, None);
    }

    #[test]
    fn a_class_guess_never_triggers_a_rule_the_user_wrote_for_a_real_id() {
        let s = settings_with_rules();
        // Focus says "org.example.Editor", but no id came from the portal: the key is class:org.example.Editor.
        assert_eq!(
            plan(&s, &resolve("", Some("org.example.Editor")), None, &any_dir).seed_folder,
            None
        );
    }

    #[test]
    fn an_unknown_app_gets_no_start_folder() {
        assert_eq!(
            plan(&settings_with_rules(), &Identity::unknown(), None, &any_dir).seed_folder,
            None
        );
    }

    #[test]
    fn a_start_folder_that_no_longer_exists_is_not_used() {
        let p = plan(
            &settings_with_rules(),
            &resolve("", Some("zen")),
            None,
            &|_| false,
        );
        assert_eq!(p.seed_folder, None);
    }

    #[test]
    fn while_paused_nothing_is_shown_or_moved() {
        let s = Settings {
            paused: true,
            ..settings_with_rules()
        };
        assert_eq!(
            plan(&s, &resolve("", Some("zen")), None, &any_dir),
            Plan {
                show_strip: false,
                seed_folder: None
            }
        );
    }

    #[test]
    fn an_excluded_app_gets_no_strip_but_everyone_else_does() {
        let s = settings_with_rules();
        assert!(!plan(&s, &resolve("", Some("kiosk")), None, &any_dir).show_strip);
        assert!(plan(&s, &resolve("", Some("zen")), None, &any_dir).show_strip);
        assert!(plan(&s, &Identity::unknown(), None, &any_dir).show_strip);
    }
}
