//! The `ofolder` command.

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

mod hooks;
mod live;
mod strip_cmd;

use ofolder_core::adapter::known_capabilities;
use ofolder_core::portal::{self, FILE_CHOOSER, dbus_name_for};
use ofolder_portal::hooks::PassThrough;

const USAGE: &str = "usage: ofolder <command>

commands:
  adapters     list the installed file-chooser backends and which ofolder can decorate
  strip-demo   open a sample strip window to see how it draws
  portal --backend <flea|gtk|strata>
               serve a file-chooser proxy that forwards to that backend
  strip --backend <id> --title <t>
               the strip for one open dialog (the proxy starts these)
  live-check [flea|gtk|strata]
               run an adapter against the real dialog (opens a window briefly)
  version      print the version";

fn main() -> anyhow::Result<()> {
    match env::args().nth(1).as_deref() {
        Some("adapters") => {
            adapters();
            Ok(())
        }
        Some("strip-demo") => ofolder_strip::run_demo(),
        Some("portal") => portal(&env::args().skip(2).collect::<Vec<_>>()),
        Some("strip") => strip_cmd::run(&env::args().skip(2).collect::<Vec<_>>()),
        Some("live-check") => live::run(env::args().nth(2).as_deref().unwrap_or("flea")),
        Some("version" | "--version") => {
            println!("ofolder {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

/// The directories the portal frontend reads `*.portal` files from, user
/// first: the same order it uses, so the same backend wins.
fn portal_dirs() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".local/share")));
    let data_dirs =
        env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    data_home
        .into_iter()
        .chain(data_dirs.split(':').map(PathBuf::from))
        .map(|d| d.join("xdg-desktop-portal/portals"))
        .collect()
}

fn adapters() {
    let backends: Vec<_> = portal::discover(&portal_dirs())
        .into_iter()
        .filter(portal::PortalFile::provides_file_chooser)
        .collect();
    if backends.is_empty() {
        println!("no {FILE_CHOOSER} backends found");
        return;
    }
    for backend in backends {
        match backend.adapter_id() {
            Some(id) => {
                let caps = known_capabilities(id);
                println!(
                    "{:<46} adapter {id:<7} read_folder={} keyless_goto={} native_new_folder={} opens_tiled={}",
                    backend.dbus_name,
                    caps.read_folder,
                    caps.goto_without_keys,
                    caps.native_new_folder,
                    caps.opens_tiled
                );
            }
            None => println!("{:<46} no adapter: left undecorated", backend.dbus_name),
        }
    }
}

/// The bus name this backend answers on. It is not in anyone's portal
/// configuration until the user adds it, so the frontend never calls it.
const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.ofolder";

fn portal(args: &[String]) -> anyhow::Result<()> {
    let (id, strip) = match args {
        [flag, id] if flag == "--backend" => (id.as_str(), true),
        [flag, id, off] if flag == "--backend" && off == "--no-strip" => (id.as_str(), false),
        _ => anyhow::bail!("usage: ofolder portal --backend <flea|gtk|strata> [--no-strip]"),
    };
    let backend = dbus_name_for(id).ok_or_else(|| anyhow::anyhow!("no adapter named {id:?}"))?;
    let hooks: Arc<dyn ofolder_portal::hooks::Hooks> = if strip {
        Arc::new(hooks::StripHooks {
            exe: env::current_exe()?,
            backend: id.to_owned(),
            compositor: Arc::new(ofolder_adapters::hyprctl::Hyprctl),
        })
    } else {
        Arc::new(PassThrough)
    };
    ofolder_portal::block_on(async {
        let _connection = ofolder_portal::serve_session(BUS_NAME, backend, hooks).await?;
        println!(
            "ofolder portal: {BUS_NAME} forwards to {backend}{}",
            if strip {
                " with a strip per dialog"
            } else {
                ""
            }
        );
        std::future::pending::<()>().await;
        Ok(())
    })
}
