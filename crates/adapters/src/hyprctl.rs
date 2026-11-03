//! The compositor, through the `hyprctl` command.

use std::process::Command;

use ofolder_core::hypr::{
    Client, Monitor, float_rule_lua, is_plain_identifier, is_window_address, parse_clients,
    parse_monitors, remove_rule_lua, tip_rule_lua,
};
use ofolder_core::system::{Compositor, SystemError};

#[derive(Debug, Default, Clone, Copy)]
pub struct Hyprctl;

fn failure(what: &str, detail: impl std::fmt::Display) -> SystemError {
    SystemError(format!("hyprctl {what}: {detail}"))
}

impl Hyprctl {
    fn run(args: &[&str]) -> Result<String, SystemError> {
        let output = Command::new("hyprctl")
            .args(args)
            .output()
            .map_err(|e| failure(&args.join(" "), e))?;
        if !output.status.success() {
            return Err(failure(
                &args.join(" "),
                String::from_utf8_lossy(&output.stderr),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Hyprctl {
    fn dispatch(what: &str, lua: &str) -> Result<(), SystemError> {
        let reply = Self::run(&["dispatch", lua])?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(failure(what, reply.trim()))
        }
    }

    fn eval(what: &str, lua: &str) -> Result<(), SystemError> {
        let reply = Self::run(&["eval", lua])?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(failure(what, reply.trim()))
        }
    }

    fn checked(address: &str) -> Result<(), SystemError> {
        if is_window_address(address) {
            Ok(())
        } else {
            Err(failure(
                "window",
                format!("refusing a malformed window address {address:?}"),
            ))
        }
    }
}

/// The pointer position from `hyprctl cursorpos -j`, in layout pixels.
fn parse_cursor(json: &str) -> Option<(i64, i64)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    Some((
        value.get("x")?.as_f64()? as i64,
        value.get("y")?.as_f64()? as i64,
    ))
}

impl Compositor for Hyprctl {
    fn clients(&self) -> Result<Vec<Client>, SystemError> {
        parse_clients(&Self::run(&["clients", "-j"])?).map_err(|e| failure("clients", e))
    }

    fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
        parse_monitors(&Self::run(&["monitors", "-j"])?).map_err(|e| failure("monitors", e))
    }

    fn active_window(&self) -> Result<Option<String>, SystemError> {
        let json: serde_json::Value = serde_json::from_str(&Self::run(&["activewindow", "-j"])?)
            .map_err(|e| failure("activewindow", e))?;
        Ok(json
            .get("address")
            .and_then(|a| a.as_str())
            .map(str::to_owned))
    }

    fn focus(&self, address: &str) -> Result<(), SystemError> {
        Self::checked(address).map_err(|_| {
            failure(
                "focus",
                format!("refusing a malformed window address {address:?}"),
            )
        })?;
        // Focusing a window warps the pointer to its centre, which pulls it
        // off the rail icon the user just clicked. Put it back afterwards;
        // moving it by dispatcher does not hand focus to the window under it.
        let pointer = Self::run(&["cursorpos", "-j"])
            .ok()
            .and_then(|json| parse_cursor(&json));
        let focused = Self::dispatch(
            "focus",
            &format!("hl.dsp.focus({{ window = \"address:{address}\" }})"),
        );
        if let Some((x, y)) = pointer {
            Self::dispatch(
                "cursor",
                &format!("hl.dsp.cursor.move({{ x = {x}, y = {y} }})"),
            )
            .ok();
        }
        focused
    }

    fn float_window(
        &self,
        address: &str,
        size: (i32, i32),
        at: (i32, i32),
    ) -> Result<(), SystemError> {
        Self::checked(address)?;
        Self::dispatch(
            "float",
            &format!("hl.dsp.window.float({{ action = \"on\", window = \"address:{address}\" }})"),
        )?;
        self.resize_window(address, size)?;
        self.move_window(address, at)
    }

    fn resize_window(&self, address: &str, size: (i32, i32)) -> Result<(), SystemError> {
        Self::checked(address)?;
        Self::dispatch(
            "resize",
            &format!(
                "hl.dsp.window.resize({{ x = {}, y = {}, relative = false, window = \"address:{address}\" }})",
                size.0, size.1
            ),
        )
    }

    fn move_window(&self, address: &str, at: (i32, i32)) -> Result<(), SystemError> {
        Self::checked(address)?;
        Self::dispatch(
            "move",
            &format!(
                "hl.dsp.window.move({{ x = {}, y = {}, relative = false, window = \"address:{address}\" }})",
                at.0, at.1
            ),
        )
    }

    fn send_to_workspace(&self, address: &str, workspace: &str) -> Result<(), SystemError> {
        Self::checked(address)?;
        if !is_plain_identifier(workspace) {
            return Err(failure(
                "workspace",
                format!("refusing a workspace name that is not plain: {workspace:?}"),
            ));
        }
        Self::dispatch(
            "workspace",
            &format!(
                "hl.dsp.window.move({{ workspace = \"{workspace}\", follow = false, window = \"address:{address}\" }})"
            ),
        )
    }

    fn install_float_rule(
        &self,
        name: &str,
        class: &str,
        size: (i32, i32),
        at: (i32, i32),
        take_focus: bool,
    ) -> Result<(), SystemError> {
        Self::eval(
            "rule",
            &float_rule_lua(name, class, size, at, take_focus).map_err(|e| failure("rule", e))?,
        )
    }

    fn install_tip_rule(
        &self,
        name: &str,
        class: &str,
        size: (i32, i32),
        at: (i32, i32),
    ) -> Result<(), SystemError> {
        Self::eval(
            "rule",
            &tip_rule_lua(name, class, size, at).map_err(|e| failure("rule", e))?,
        )
    }

    fn remove_rule(&self, name: &str) -> Result<(), SystemError> {
        Self::eval(
            "rule",
            &remove_rule_lua(name).map_err(|e| failure("rule", e))?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_window_operation_refuses_a_malformed_address_before_running_anything() {
        let bad = "0x1\" })";
        assert!(Hyprctl.float_window(bad, (1, 1), (0, 0)).is_err());
        assert!(Hyprctl.resize_window(bad, (1, 1)).is_err());
        assert!(Hyprctl.move_window(bad, (0, 0)).is_err());
        assert!(Hyprctl.send_to_workspace(bad, "1").is_err());
        assert!(Hyprctl.send_to_workspace("0x1", "1\" })").is_err());
        assert!(
            Hyprctl
                .install_float_rule("a b", "c", (1, 1), (0, 0), true)
                .is_err()
        );
    }

    #[test]
    fn the_pointer_position_is_read_from_the_cursorpos_reply() {
        assert_eq!(
            parse_cursor("{\n\t\"x\": 2099,\n\t\"y\": 984\n}"),
            Some((2099, 984))
        );
        assert_eq!(parse_cursor("{\"x\": 10.6, \"y\": 3.2}"), Some((10, 3)));
        assert_eq!(parse_cursor("not json"), None);
        assert_eq!(parse_cursor("{\"x\": 1}"), None);
    }

    #[test]
    fn focus_refuses_an_address_that_could_break_out_of_the_lua_string() {
        let err = Hyprctl
            .focus("0x1\" }) os.execute(\"x\") --")
            .expect_err("must refuse");
        assert!(err.0.contains("malformed window address"), "{err}");
    }
}
