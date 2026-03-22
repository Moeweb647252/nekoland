use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::fd::{AsFd, AsRawFd, RawFd};

use bevy_app::App;
use bevy_ecs::prelude::ResMut;
use calloop::generic::{FdWrapper, Generic};
use calloop::{Interest, Mode, PostAction};
use nekoland_core::calloop::with_wayland_calloop_registry;
use nekoland_core::error::NekolandError;

#[derive(Debug, Clone, bevy_ecs::prelude::Resource)]
pub(crate) struct ProtocolBootstrapConfig {
    pub(crate) repeat_rate: u16,
    pub(crate) initial_keyboard_layout: nekoland_config::resources::ConfiguredKeyboardLayout,
    pub(crate) xwayland_enabled: bool,
}

impl Default for ProtocolBootstrapConfig {
    fn default() -> Self {
        Self {
            repeat_rate: super::DEFAULT_KEYBOARD_REPEAT_RATE,
            initial_keyboard_layout: nekoland_config::resources::ConfiguredKeyboardLayout::default(
            ),
            xwayland_enabled: true,
        }
    }
}

pub(crate) fn bootstrap_protocol_runtime_in_subapp(app: &mut App) {
    let bootstrap =
        app.world().get_resource::<ProtocolBootstrapConfig>().cloned().unwrap_or_default();
    let (server, server_state) = super::server::SmithayProtocolServer::new(
        bootstrap.repeat_rate,
        bootstrap.initial_keyboard_layout,
        bootstrap.xwayland_enabled,
    );
    register_calloop_sources(app, &server);
    app.insert_non_send_resource(server);
    app.insert_non_send_resource(crate::ProtocolSurfaceRegistry::default());
    app.insert_non_send_resource(super::server::ProtocolCursorState::default());
    app.insert_resource(server_state);
}

pub(crate) fn advance_compositor_clock_system(
    mut clock: ResMut<'_, nekoland_ecs::resources::CompositorClock>,
    mut started_at: bevy_ecs::prelude::Local<Option<std::time::Instant>>,
) {
    let started_at = started_at.get_or_insert_with(std::time::Instant::now);
    clock.frame = clock.frame.saturating_add(1);
    clock.uptime_millis = started_at.elapsed().as_millis();
}

pub(crate) fn bind_wayland_socket()
-> std::io::Result<(smithay::reexports::wayland_server::ListeningSocket, OsString)> {
    let _runtime_dir_guard = RuntimeDirGuard::install()?;

    match smithay::reexports::wayland_server::ListeningSocket::bind_auto("wayland", 0..33) {
        Ok(socket) => {
            let socket_name =
                OsString::from(socket_name_or_default(socket_name_or_none_ref(&socket), "wayland"));
            Ok((socket, socket_name))
        }
        Err(auto_error) => {
            let fallback_name = format!("nekoland-{}", std::process::id());
            match smithay::reexports::wayland_server::ListeningSocket::bind(&fallback_name) {
                Ok(socket) => Ok((socket, OsString::from(fallback_name))),
                Err(fallback_error) => Err(std::io::Error::other(format!(
                    "auto socket failed ({auto_error}); fallback socket `{fallback_name}` failed ({fallback_error})"
                ))),
            }
        }
    }
}

fn socket_name_or_none_ref(
    socket: &smithay::reexports::wayland_server::ListeningSocket,
) -> Option<&OsStr> {
    socket.socket_name()
}

fn socket_name_or_default(name: Option<&OsStr>, fallback: &str) -> String {
    name.unwrap_or_else(|| OsStr::new(fallback)).to_string_lossy().into_owned()
}

#[derive(Debug)]
struct RuntimeDirGuard {
    previous: Option<OsString>,
}

impl RuntimeDirGuard {
    fn install() -> std::io::Result<Option<Self>> {
        let Some(runtime_dir) = env::var_os("NEKOLAND_RUNTIME_DIR") else {
            return Ok(None);
        };

        fs::create_dir_all(&runtime_dir)?;
        let previous = env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            env::set_var("XDG_RUNTIME_DIR", &runtime_dir);
        }

        tracing::info!(
            runtime_dir = %display_runtime_dir(&runtime_dir),
            "using overridden Wayland runtime dir"
        );
        Ok(Some(Self { previous }))
    }
}

impl Drop for RuntimeDirGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => unsafe {
                env::set_var("XDG_RUNTIME_DIR", previous);
            },
            None => unsafe {
                env::remove_var("XDG_RUNTIME_DIR");
            },
        }
    }
}

fn display_runtime_dir(path: &OsStr) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn current_wayland_runtime_dir() -> Option<String> {
    env::var_os("NEKOLAND_RUNTIME_DIR")
        .or_else(|| env::var_os("XDG_RUNTIME_DIR"))
        .map(|path| path.to_string_lossy().into_owned())
}

pub(crate) fn register_calloop_sources(
    app: &mut App,
    server: &super::server::SmithayProtocolServer,
) {
    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    let runtime = runtime.clone();
    let display_fd = runtime.borrow().display.as_fd().as_raw_fd();
    let socket_fd = runtime.borrow().socket.as_ref().map(AsRawFd::as_raw_fd);

    with_wayland_calloop_registry(app, |registry| {
        registry.push(move |handle| {
            let display_runtime = runtime.clone();
            handle
                .insert_source(
                    Generic::new(
                        unsafe { FdWrapper::new(RegisteredRawFd(display_fd)) },
                        Interest::READ,
                        Mode::Level,
                    ),
                    move |_, _, _| {
                        display_runtime.borrow_mut().on_display_ready();
                        Ok(PostAction::Continue)
                    },
                )
                .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

            if let Some(socket_fd) = socket_fd {
                let socket_runtime = runtime.clone();
                handle
                    .insert_source(
                        Generic::new(
                            unsafe { FdWrapper::new(RegisteredRawFd(socket_fd)) },
                            Interest::READ,
                            Mode::Level,
                        ),
                        move |_, _, _| {
                            socket_runtime.borrow_mut().on_socket_ready();
                            Ok(PostAction::Continue)
                        },
                    )
                    .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;
            }

            Ok(())
        });
    });
}

#[derive(Debug, Clone, Copy)]
struct RegisteredRawFd(RawFd);

impl AsRawFd for RegisteredRawFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}
