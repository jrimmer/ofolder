//! `ofolder live-check <flea|gtk|strata>`: runs an adapter against the real
//! dialog on the real desktop, to prove what the unit tests can only fake.
//! It opens a dialog window for a few seconds and works only in scratch
//! folders, apart from moving the dialog to the home folder to look at it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use ofolder_adapters::flea::FleaAdapter;
use ofolder_adapters::gtk::GtkAdapter;
use ofolder_adapters::hyprctl::Hyprctl;
use ofolder_adapters::strata::StrataAdapter;
use ofolder_adapters::wtype::Wtype;
use ofolder_core::adapter::{Adapter, DialogRequest, goto_verified, stable_folder};
use ofolder_core::system::{Compositor, Typist};

const SUGGESTED: &str = "live.txt";

/// Reads the file name an adapter's dialog is holding.
type NameReader<'a, A> = &'a dyn Fn(&A) -> Option<String>;

fn wait_for(limit: Duration, mut condition: impl FnMut() -> bool) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if condition() {
            return Some(start.elapsed());
        }
        sleep(Duration::from_millis(20));
    }
    None
}

struct Report {
    failures: u32,
}

impl Report {
    fn check(&mut self, name: &str, took: Option<Duration>) {
        match took {
            Some(t) => println!("PASS {name} ({} ms)", t.as_millis()),
            None => {
                println!("FAIL {name}");
                self.failures += 1;
            }
        }
    }

    fn truth(&mut self, name: &str, ok: bool) {
        self.check(name, ok.then_some(Duration::ZERO));
    }
}

pub fn run(which: &str) -> anyhow::Result<()> {
    let scratch = std::env::temp_dir().join(format!("ofolder-live-{}", std::process::id()));
    let names = ["one", "two", "with space", "ünïcödé 日本語"];
    let folders: Vec<PathBuf> = names.iter().map(|n| scratch.join(n)).collect();
    for folder in &folders {
        fs::create_dir_all(folder).context("scratch folder")?;
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let compositor: Arc<dyn Compositor> = Arc::new(Hyprctl);
    let typist: Arc<dyn Typist> = Arc::new(Wtype);
    let title = format!("ofolder live check {}", std::process::id());
    let result = match which {
        "flea" => run_adapter(
            &mut FleaAdapter::new(compositor, typist),
            &folders,
            &home,
            None,
        ),
        "gtk" => {
            let mut adapter =
                GtkAdapter::new(compositor, &title).map_err(|e| anyhow::anyhow!("{e}"))?;
            run_adapter(
                &mut adapter,
                &folders,
                &home,
                Some(&|a: &GtkAdapter| a.suggested_name()),
            )
        }
        "strata" => {
            let mut adapter = StrataAdapter::new(compositor, typist, &title, home.clone())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            run_adapter(&mut adapter, &folders, &home, None)
        }
        other => bail!("unknown adapter {other:?}; use flea, gtk or strata"),
    };
    // Scratch only: nothing under it is the user's.
    fs::remove_dir_all(&scratch).ok();
    result
}

/// `name_of` reads the file name the dialog is holding, for adapters that borrow it.
fn run_adapter<A: Adapter>(
    adapter: &mut A,
    folders: &[PathBuf],
    home: &Path,
    name_of: Option<NameReader<'_, A>>,
) -> anyhow::Result<()> {
    let mut report = Report { failures: 0 };
    adapter
        .launch(&DialogRequest {
            folder: folders[0].clone(),
            suggested_name: SUGGESTED.to_owned(),
            title: format!("ofolder live check {}", std::process::id()),
        })
        .context("launch")?;
    report.check(
        "the dialog's window is found",
        wait_for(Duration::from_secs(10), || adapter.window().is_some()),
    );
    let reads = |a: &A, folder: &PathBuf| {
        stable_folder(a, 6, Duration::from_millis(60)).as_deref() == Some(folder.as_path())
    };
    report.check(
        "the folder it was started in reads back",
        wait_for(Duration::from_secs(8), || reads(adapter, &folders[0])),
    );
    let steps = folders[1..]
        .iter()
        .cloned()
        .chain([home.to_path_buf()])
        .collect::<Vec<_>>();
    for folder in &steps {
        let label = folder
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_owned();
        let arrived = match goto_verified(adapter, folder, Duration::from_secs(5)) {
            Ok(took) => Some(took),
            Err(error) => {
                println!("goto {label}: {error}");
                None
            }
        };
        report.check(&format!("goto '{label}' arrives and reads back"), arrived);
    }
    if let Some(name_of) = name_of {
        report.truth(
            "the application's suggested file name survived every move",
            name_of(adapter).as_deref() == Some(SUGGESTED),
        );
    }
    adapter.close().context("close")?;
    report.check(
        "closing ends the dialog",
        wait_for(Duration::from_secs(5), || !adapter.alive()),
    );
    if report.failures > 0 {
        bail!("{} live check(s) failed", report.failures);
    }
    println!("all live checks passed");
    Ok(())
}
