//! The D-Bus objects: the file chooser that forwards, and the request object
//! the frontend uses to cancel.

use std::sync::Arc;

use zbus::connection::Builder;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};
use zbus::{Connection, Proxy, fdo, interface};

use crate::PortalError;
use crate::hooks::{Call, Hooks, Method, Options, response};

/// Where every portal backend exports its interfaces.
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.impl.portal.Request";

struct FileChooser {
    backend: String,
    hooks: Arc<dyn Hooks>,
}

/// The request object for one call. The frontend calls `Close` on it when the
/// application cancels, and that has to reach the dialog the real backend
/// opened, which is on the same path under the real backend's own name.
struct RequestObject {
    backend: String,
    handle: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestObject {
    async fn close(&self, #[zbus(connection)] connection: &Connection) -> fdo::Result<()> {
        let proxy = Proxy::new(connection, self.backend.as_str(), &self.handle, REQUEST)
            .await
            .map_err(fdo::Error::from)?;
        proxy
            .call::<_, _, ()>("Close", &())
            .await
            .map_err(fdo::Error::from)
    }
}

impl FileChooser {
    async fn run(
        &self,
        connection: &Connection,
        call: Call,
        handle: &ObjectPath<'_>,
        mut options: Options,
    ) -> fdo::Result<(u32, Options)> {
        let guard = self.hooks.begin(&call, &mut options);
        let path = OwnedObjectPath::from(handle.to_owned());
        let request = RequestObject {
            backend: self.backend.clone(),
            handle: path.clone(),
        };
        // Registered before forwarding so a cancel that arrives at once is
        // not lost. It fails if the frontend reuses a path that is in use.
        let registered = connection
            .object_server()
            .at(&path, request)
            .await
            .map_err(fdo::Error::from)?;
        if !registered {
            return Err(fdo::Error::Failed(format!(
                "request {} is already in use",
                call.handle
            )));
        }

        let forwarded: zbus::Result<(u32, Options)> = async {
            let proxy =
                Proxy::new(connection, self.backend.as_str(), PORTAL_PATH, FILE_CHOOSER).await?;
            proxy
                .call(
                    call.method.dbus_name(),
                    &(
                        handle,
                        call.app_id.as_str(),
                        call.parent_window.as_str(),
                        call.title.as_str(),
                        &options,
                    ),
                )
                .await
        }
        .await;

        // The request is over either way. It may already be gone if the
        // connection dropped, which leaves nothing to remove.
        connection
            .object_server()
            .remove::<RequestObject, _>(&path)
            .await
            .ok();

        let (code, results) = match forwarded {
            Ok(reply) => reply,
            // An unreachable backend is an "other" answer, which the frontend
            // reports to the application, not a crash of this one.
            Err(_) => (response::OTHER, Options::new()),
        };
        self.hooks.end(&call, code, &results);
        drop(guard);
        Ok((code, results))
    }

    fn call(
        method: Method,
        handle: &ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
    ) -> Call {
        Call {
            method,
            handle: handle.to_string(),
            app_id: app_id.to_owned(),
            parent_window: parent_window.to_owned(),
            title: title.to_owned(),
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    async fn open_file(
        &self,
        #[zbus(connection)] connection: &Connection,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: Options,
    ) -> fdo::Result<(u32, Options)> {
        let call = Self::call(Method::OpenFile, &handle, app_id, parent_window, title);
        self.run(connection, call, &handle, options).await
    }

    async fn save_file(
        &self,
        #[zbus(connection)] connection: &Connection,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: Options,
    ) -> fdo::Result<(u32, Options)> {
        let call = Self::call(Method::SaveFile, &handle, app_id, parent_window, title);
        self.run(connection, call, &handle, options).await
    }

    async fn save_files(
        &self,
        #[zbus(connection)] connection: &Connection,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: Options,
    ) -> fdo::Result<(u32, Options)> {
        let call = Self::call(Method::SaveFiles, &handle, app_id, parent_window, title);
        self.run(connection, call, &handle, options).await
    }
}

/// Exports the proxy on `builder`'s bus under `bus_name`, forwarding to the
/// backend named `backend`. The returned connection keeps the service alive.
pub async fn serve(
    builder: Builder<'_>,
    bus_name: &str,
    backend: &str,
    hooks: Arc<dyn Hooks>,
) -> Result<Connection, PortalError> {
    let chooser = FileChooser {
        backend: backend.to_owned(),
        hooks,
    };
    // Objects first, name last: nothing can call the service before it can answer.
    Ok(builder
        .serve_at(PORTAL_PATH, chooser)?
        .name(bus_name.to_owned())?
        .build()
        .await?)
}

/// The names the session bus knows: those with an owner now, and those a service would be started
/// under. A check reads them and claims nothing.
pub async fn session_bus_names() -> Result<Vec<String>, PortalError> {
    let connection = Connection::session().await?;
    let bus = fdo::DBusProxy::new(&connection).await?;
    let running = bus.list_names().await.map_err(zbus::Error::from)?;
    let activatable = bus
        .list_activatable_names()
        .await
        .map_err(zbus::Error::from)?;
    Ok(running
        .into_iter()
        .chain(activatable)
        .map(|name| name.to_string())
        .collect())
}

/// [`serve`] on the user's session bus, which is where a real portal backend runs.
pub async fn serve_session(
    bus_name: &str,
    backend: &str,
    hooks: Arc<dyn Hooks>,
) -> Result<Connection, PortalError> {
    serve(Builder::session()?, bus_name, backend, hooks).await
}
