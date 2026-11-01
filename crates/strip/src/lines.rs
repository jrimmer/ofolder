//! Turning a list of folders, with what is open under them, into the flat
//! lines a panel draws. No window types here, so it is testable.

use std::path::{Path, PathBuf};

use crate::expand::{Expansion, Node, RowKey, Section};
use crate::favorites::Favorites;
use crate::menu::{RowAction, RowMenu, actions_for};
use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Folder,
    Drive,
    Home,
}

/// A folder to list, before anything is opened under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub path: PathBuf,
    pub label: String,
    pub icon: Icon,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderLine {
    pub key: RowKey,
    pub depth: usize,
    pub label: String,
    /// The path shortened to fit, for top-level rows only.
    pub subtitle: Option<String>,
    pub full_path: String,
    pub missing: bool,
    pub is_current: bool,
    pub open: bool,
    pub menu_open: bool,
    pub highlighted: bool,
    pub icon: Icon,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Folder(FolderLine),
    File {
        path: PathBuf,
        label: String,
        subtitle: String,
    },
    Loading(usize),
    Failed(usize, String),
    Empty(usize),
    Menu {
        key: RowKey,
        depth: usize,
        items: Vec<(RowAction, String)>,
    },
    RenamePrompt(usize),
    /// "Show all" or "Show fewer" for a list cut at a limit.
    More {
        section: Section,
        hidden: usize,
    },
}

/// What flattening needs to know about the rest of the program.
pub struct Context<'a> {
    pub section: Section,
    pub expansion: &'a Expansion,
    pub menu: &'a RowMenu,
    pub favorites: &'a Favorites,
    pub current: Option<&'a Path>,
    pub home: &'a Path,
    pub missing: &'a dyn Fn(&Path) -> bool,
    /// The folder being renamed, whose prompt goes under its row.
    pub renaming: Option<&'a Path>,
    /// Index into the roots to draw as the highlighted one.
    pub highlight: Option<usize>,
}

/// How many characters of path fit under a label.
const SUBTITLE_CHARS: usize = 26;

pub fn flatten(roots: &[Root], ctx: &Context<'_>) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        push_folder(&mut out, ctx, root, 0, ctx.highlight == Some(i));
    }
    out
}

fn push_folder(out: &mut Vec<Line>, ctx: &Context<'_>, root: &Root, depth: usize, hi: bool) {
    let key = RowKey::new(ctx.section, root.path.clone());
    let open = ctx.expansion.is_open(&key);
    let is_favorite = ctx.favorites.contains(&root.path);
    out.push(Line::Folder(FolderLine {
        key: key.clone(),
        depth,
        label: root.label.clone(),
        subtitle: (depth == 0)
            .then(|| paths::shorten_path(&paths::abbreviate(&root.path, ctx.home), SUBTITLE_CHARS)),
        full_path: root.path.display().to_string(),
        missing: (ctx.missing)(&root.path),
        is_current: ctx.current == Some(root.path.as_path()),
        open,
        menu_open: ctx.menu.is_open(&key),
        highlighted: hi,
        icon: root.icon,
    }));
    if ctx.menu.is_open(&key) {
        out.push(Line::Menu {
            key: key.clone(),
            depth,
            items: actions_for(ctx.section, is_favorite)
                .into_iter()
                .map(|(a, l)| (a, l.to_owned()))
                .collect(),
        });
    }
    if ctx.renaming == Some(root.path.as_path()) {
        out.push(Line::RenamePrompt(depth));
    }
    if !open {
        return;
    }
    match ctx.expansion.node(&key) {
        Some(Node::Loading) | None => out.push(Line::Loading(depth + 1)),
        Some(Node::Failed(why)) => out.push(Line::Failed(depth + 1, why.clone())),
        Some(Node::Loaded(children)) if children.is_empty() => {
            out.push(Line::Empty(depth + 1));
        }
        Some(Node::Loaded(children)) => {
            for child in children {
                let label = child.file_name().map_or_else(
                    || child.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let sub = Root {
                    path: child.clone(),
                    label,
                    icon: Icon::Folder,
                };
                push_folder(out, ctx, &sub, depth + 1, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        expansion: Expansion,
        menu: RowMenu,
        favorites: Favorites,
    }

    fn fixture() -> Fixture {
        Fixture {
            expansion: Expansion::default(),
            menu: RowMenu::default(),
            favorites: Favorites::default(),
        }
    }

    fn lines(f: &Fixture, roots: &[Root], renaming: Option<&Path>) -> Vec<Line> {
        let none = |_: &Path| false;
        flatten(
            roots,
            &Context {
                section: Section::Places,
                expansion: &f.expansion,
                menu: &f.menu,
                favorites: &f.favorites,
                current: Some(Path::new("/b")),
                home: Path::new("/home/ann"),
                missing: &none,
                renaming,
                highlight: None,
            },
        )
    }

    fn root(path: &str) -> Root {
        Root {
            path: path.into(),
            label: path.trim_start_matches('/').into(),
            icon: Icon::Folder,
        }
    }

    fn key(path: &str) -> RowKey {
        RowKey::new(Section::Places, path)
    }

    #[test]
    fn closed_rows_give_one_line_each_and_flag_the_current_folder() {
        let f = fixture();
        let out = lines(&f, &[root("/a"), root("/b")], None);
        assert_eq!(out.len(), 2);
        let Line::Folder(second) = &out[1] else {
            panic!("expected a folder line");
        };
        assert!(second.is_current);
        assert_eq!(second.subtitle.as_deref(), Some("/b"));
    }

    #[test]
    fn an_opening_row_shows_loading_one_level_deeper() {
        let mut f = fixture();
        f.expansion.toggle(key("/a"));
        let out = lines(&f, &[root("/a")], None);
        assert_eq!(out[1], Line::Loading(1));
    }

    #[test]
    fn a_failed_read_shows_its_error_text_under_the_row() {
        let mut f = fixture();
        f.expansion.toggle(key("/a"));
        f.expansion.finish(&key("/a"), Err("cannot read /a".into()));
        let out = lines(&f, &[root("/a")], None);
        assert_eq!(out[1], Line::Failed(1, "cannot read /a".into()));
    }

    #[test]
    fn an_empty_folder_says_so() {
        let mut f = fixture();
        f.expansion.toggle(key("/a"));
        f.expansion.finish(&key("/a"), Ok(vec![]));
        assert_eq!(lines(&f, &[root("/a")], None)[1], Line::Empty(1));
    }

    #[test]
    fn subfolders_nest_to_any_depth_and_each_level_is_a_row_of_its_own() {
        let mut f = fixture();
        f.expansion.toggle(key("/a"));
        f.expansion.finish(&key("/a"), Ok(vec!["/a/b".into()]));
        f.expansion.toggle(key("/a/b"));
        f.expansion.finish(&key("/a/b"), Ok(vec!["/a/b/c".into()]));
        let out = lines(&f, &[root("/a")], None);
        let depths: Vec<usize> = out
            .iter()
            .filter_map(|l| match l {
                Line::Folder(l) => Some(l.depth),
                _ => None,
            })
            .collect();
        assert_eq!(depths, [0, 1, 2]);
        let Line::Folder(deepest) = &out[2] else {
            panic!("expected a folder line");
        };
        assert_eq!(deepest.label, "c");
        assert_eq!(deepest.subtitle, None);
    }

    #[test]
    fn an_open_menu_appears_right_under_its_row() {
        let mut f = fixture();
        f.menu.toggle(key("/a"));
        let out = lines(&f, &[root("/a"), root("/b")], None);
        assert!(matches!(out[1], Line::Menu { .. }));
        assert!(matches!(&out[2], Line::Folder(l) if l.label == "b"));
    }

    #[test]
    fn the_menu_offers_remove_for_a_folder_that_is_already_a_favorite() {
        let mut f = fixture();
        f.favorites.add("/a".into());
        f.menu.toggle(key("/a"));
        let out = lines(&f, &[root("/a")], None);
        let Line::Menu { items, .. } = &out[1] else {
            panic!("expected a menu");
        };
        assert!(items.iter().any(|i| i.1 == "Remove favorite"));
    }

    #[test]
    fn the_rename_prompt_goes_under_the_favorite_being_renamed() {
        let f = fixture();
        let out = lines(&f, &[root("/a"), root("/b")], Some(Path::new("/a")));
        assert_eq!(out[1], Line::RenamePrompt(0));
    }
}
