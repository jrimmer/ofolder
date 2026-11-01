//! What the portal proxy does around each dialog: start a strip for it, and
//! make sure it is gone when the dialog is.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use ofolder_adapters::flea::picker_pids;
use ofolder_adapters::strata;
use ofolder_core::system::Compositor;
use ofolder_portal::hooks::{Call, Hooks, Options};

pub struct StripHooks {
    /// The `ofolder` binary to run for each strip.
    pub exe: PathBuf,
    /// The adapter id of the backend the proxy forwards to.
    pub backend: String,
    pub compositor: Arc<dyn Compositor>,
}

/// Held for the life of one dialog; dropping it stops the strip and undoes
/// anything installed for the dialog.
struct DialogGuard {
    strip: Option<Child>,
    compositor: Arc<dyn Compositor>,
    strata_rule: bool,
}

impl Drop for DialogGuard {
    fn drop(&mut self) {
        if let Some(strip) = self.strip.as_mut() {
            // The strip exits by itself when the dialog closes; this covers a crash or a hang.
            strip.kill().ok();
            strip.wait().ok();
        }
        if self.strata_rule {
            strata::remove_float_rule(&*self.compositor);
        }
    }
}

impl Hooks for StripHooks {
    fn begin(&self, call: &Call, _options: &mut Options) -> Box<dyn Send> {
        // Strata's dialog tiles unless a rule says otherwise, and the rule has to exist before it opens.
        let strata_rule =
            self.backend == "strata" && strata::install_float_rule(&*self.compositor).is_ok();
        let known: Vec<String> = if self.backend == "flea" {
            picker_pids(&*self.compositor)
                .iter()
                .map(u32::to_string)
                .collect()
        } else {
            Vec::new()
        };
        let strip = Command::new(&self.exe)
            .args([
                "strip",
                "--backend",
                &self.backend,
                "--title",
                &call.title,
                "--known-pids",
                &known.join(","),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| eprintln!("ofolder: could not start the strip: {e}"))
            .ok();
        Box::new(DialogGuard {
            strip,
            compositor: self.compositor.clone(),
            strata_rule,
        })
    }
}
