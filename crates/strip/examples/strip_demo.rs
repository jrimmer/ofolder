//! Opens the strip against a fake dialog. Usage: strip_demo <scratch folder>
//! The folder must exist; the favorites file is written inside it.

use std::path::PathBuf;
use std::sync::Arc;

use ofolder_strip::{DemoActions, StripOptions, favorites, run_strip};

fn main() -> anyhow::Result<()> {
    let base = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("give a scratch folder"))?,
    );
    let work = base.join("Work/ofolder");
    let mut samples = favorites::Favorites::default();
    for sample in [
        "Documents",
        "Downloads",
        "Work/ofolder",
        "Work/a-very-long-folder-name-for-the-strip",
    ] {
        std::fs::create_dir_all(base.join(sample))?;
        samples.add(base.join(sample));
    }
    samples.add(base.join("Deleted"));
    let favorites_path = base.join("favorites.json");
    favorites::save(&favorites_path, &samples)?;
    run_strip(
        StripOptions {
            label: "gtk".to_owned(),
            favorites_path,
            home: base,
        },
        Arc::new(DemoActions::new(work)),
    )
}
