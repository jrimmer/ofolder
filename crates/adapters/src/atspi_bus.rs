//! The real accessibility bus, through the `atspi` crate.
//!
//! Every property is one D-Bus round trip, so callers read only what they
//! need. Any failure reads as "nothing there": the dialog may be rebuilding
//! its widgets, and the adapters treat an unreadable tree as unknown.

use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::AccessibleProxy;
use atspi::proxy::action::ActionProxy;
use atspi::proxy::editable_text::EditableTextProxy;
use atspi::proxy::text::TextProxy;
use atspi::{ObjectRefOwned, State};
use zbus::proxy::CacheProperties;
use zbus::{Connection, block_on};

use crate::a11y::{Node, Role, Tree};

pub struct AtspiTree {
    connection: Connection,
}

impl AtspiTree {
    pub fn connect() -> Result<Self, String> {
        let conn = block_on(AccessibilityConnection::new())
            .map_err(|e| format!("accessibility bus: {e}"))?;
        Ok(Self {
            connection: conn.connection().clone(),
        })
    }
}

#[derive(Clone)]
pub struct AtspiNode {
    connection: Connection,
    bus_name: String,
    path: String,
}

fn node_of(connection: &Connection, reference: &ObjectRefOwned) -> Option<AtspiNode> {
    if reference.is_null() {
        return None;
    }
    Some(AtspiNode {
        connection: connection.clone(),
        bus_name: reference.name_as_str()?.to_owned(),
        path: reference.path_as_str().to_owned(),
    })
}

macro_rules! proxy {
    ($proxy:ident, $node:expr) => {
        block_on(async {
            $proxy::builder(&$node.connection)
                .destination($node.bus_name.clone())?
                .path($node.path.clone())?
                .cache_properties(CacheProperties::No)
                .build()
                .await
        })
        .ok()
    };
}

impl AtspiNode {
    fn accessible(&self) -> Option<AccessibleProxy<'static>> {
        proxy!(AccessibleProxy, self)
    }
}

impl Node for AtspiNode {
    fn role(&self) -> Role {
        let Some(role) = self.accessible().and_then(|p| block_on(p.get_role()).ok()) else {
            return Role::Other;
        };
        match role {
            atspi::Role::Button => Role::Button,
            atspi::Role::ToggleButton => Role::ToggleButton,
            atspi::Role::Label => Role::Label,
            atspi::Role::Text | atspi::Role::Entry => Role::Text,
            atspi::Role::List => Role::List,
            atspi::Role::ListItem => Role::ListItem,
            atspi::Role::Table => Role::Table,
            atspi::Role::TableCell => Role::TableCell,
            atspi::Role::TableColumnHeader | atspi::Role::ColumnHeader => Role::ColumnHeader,
            atspi::Role::FileChooser => Role::FileChooser,
            atspi::Role::Filler => Role::Filler,
            _ => Role::Other,
        }
    }

    fn name(&self) -> String {
        self.accessible()
            .and_then(|p| block_on(p.name()).ok())
            .unwrap_or_default()
    }

    fn checked(&self) -> bool {
        self.accessible()
            .and_then(|p| block_on(p.get_state()).ok())
            .is_some_and(|s| s.contains(State::Checked))
    }

    fn editable(&self) -> bool {
        self.accessible()
            .and_then(|p| block_on(p.get_state()).ok())
            .is_some_and(|s| s.contains(State::Editable))
    }

    fn selected(&self) -> bool {
        self.accessible()
            .and_then(|p| block_on(p.get_state()).ok())
            .is_some_and(|s| s.contains(State::Selected))
    }

    fn children(&self) -> Vec<Self> {
        let Some(refs) = self
            .accessible()
            .and_then(|p| block_on(p.get_children()).ok())
        else {
            return Vec::new();
        };
        refs.iter()
            .filter_map(|r| node_of(&self.connection, r))
            .collect()
    }

    fn parent(&self) -> Option<Self> {
        let reference = self.accessible().and_then(|p| block_on(p.parent()).ok())?;
        node_of(&self.connection, &reference)
    }

    fn press(&self) -> bool {
        proxy!(ActionProxy, self)
            .and_then(|p| block_on(p.do_action(0)).ok())
            .unwrap_or(false)
    }

    fn text(&self) -> Option<String> {
        let p = proxy!(TextProxy, self)?;
        let count = block_on(p.character_count()).ok()?;
        block_on(p.get_text(0, count)).ok()
    }

    fn set_text(&self, text: &str) -> bool {
        proxy!(EditableTextProxy, self)
            .and_then(|p| block_on(p.set_text_contents(text)).ok())
            .unwrap_or(false)
    }
}

impl Tree for AtspiTree {
    type N = AtspiNode;

    fn applications(&self) -> Vec<(String, AtspiNode)> {
        let root = AtspiNode {
            connection: self.connection.clone(),
            bus_name: "org.a11y.atspi.Registry".to_owned(),
            path: "/org/a11y/atspi/accessible/root".to_owned(),
        };
        root.children()
            .into_iter()
            .map(|app| (app.name(), app))
            .collect()
    }
}
