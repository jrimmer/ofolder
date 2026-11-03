//! Prints what an adapter reads as the selected item, a few times a second,
//! for a dialog that is already open. Read-only: nothing is typed or pressed.
//!
//! `selection_probe flea <picker pid>` or `selection_probe gtk "<dialog title>"`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ofolder_adapters::flea::FleaAdapter;
use ofolder_adapters::gtk::GtkAdapter;
use ofolder_adapters::hyprctl::Hyprctl;
use ofolder_adapters::wtype::Wtype;
use ofolder_core::adapter::Adapter;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(kind), Some(target)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: selection_probe flea <pid> | gtk <title>");
        std::process::exit(2);
    };
    let adapter: Box<dyn Adapter> = match kind.as_str() {
        "flea" => Box::new(
            FleaAdapter::new(Arc::new(Hyprctl), Arc::new(Wtype))
                .adopting(target.parse().expect("a pid")),
        ),
        "gtk" => Box::new(GtkAdapter::attach(Arc::new(Hyprctl), target).expect("the dialog")),
        other => panic!("unknown kind {other}"),
    };
    for _ in 0..args.get(3).and_then(|n| n.parse().ok()).unwrap_or(5) {
        let start = Instant::now();
        let folder = adapter.read_folder();
        let read = Instant::now();
        let selected = adapter.selection();
        println!(
            "folder {:?} ({} ms)  selection {:?} ({} ms)",
            folder,
            (read - start).as_millis(),
            selected,
            read.elapsed().as_millis()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
