//! Opens both rails against a fake dialog. Usage:
//! `strip_demo <scratch folder> [preview [light]] [favorites|places|recent|files|find|new]`
//! The folder must exist; sample favorites, recents and settings are written
//! inside it. With `preview` some rows are opened and a search is shown; a
//! menu name opens that menu.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ofolder_core::theme::Palette;
use ofolder_core::{browse, search};
use ofolder_strip::expand::{RowKey, Section};
use ofolder_strip::model::ViewModel;
use ofolder_strip::{DemoActions, MenuKind, demo_setup, run_strip_preview};

fn open(vm: &mut ViewModel, section: Section, path: PathBuf) {
    let key = RowKey::new(section, path.clone());
    if let Some(key) = vm.toggle_expand(key) {
        vm.finish_browse(&key, browse::subfolders(&path, false));
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let base = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("give a scratch folder"))?,
    );
    let rest: Vec<String> = args.collect();
    let has = |word: &str| rest.iter().any(|a| a == word);
    let preview = has("preview");
    let menu = rest.iter().find_map(|a| match a.as_str() {
        "favorites" => Some(MenuKind::Favorites),
        "places" => Some(MenuKind::Places),
        "recent" => Some(MenuKind::RecentFolders),
        "files" => Some(MenuKind::RecentFiles),
        "find" => Some(MenuKind::Find),
        "new" => Some(MenuKind::NewFolder),
        _ => None,
    });
    let palette = if has("light") {
        Palette {
            dark: false,
            background: 0xf5_f1_e8,
            surface: 0xe3_dd_cc,
            foreground: 0x2b_2a_27,
            muted: 0x6e_6a_5f,
            accent: 0x1d_6f_a5,
            selection: 0xd2_ca_b4,
            warning: 0x9a_6a_00,
            danger: 0xb3_26_1e,
        }
    } else {
        Palette::default()
    };
    let (options, work) = demo_setup(&base, palette)?;
    let start = work.clone();
    run_strip_preview(options, Arc::new(DemoActions::new(work)), move |vm| {
        if !preview {
            return menu;
        }
        vm.set_folder(Some(start.clone()));
        open(vm, Section::Favorites, start.clone());
        open(vm, Section::Favorites, start.join("docs"));
        open(vm, Section::Favorites, base.join("Deleted"));
        vm.toggle_menu(RowKey::new(Section::Favorites, base.join("Documents")));
        vm.find_set_query("doc");
        if let Some(job) = vm.start_search() {
            let cancel = AtomicBool::new(false);
            let outcome = search::search(&base, "doc", search::Limits::default(), &cancel);
            let found = outcome.found.into_iter().map(|f| f.path).collect();
            vm.finish_search(job.id, found, outcome.ended);
        }
        menu
    })
}
