#![allow(dead_code)]

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

#[derive(Debug)]
pub struct ClientSummary {
    pub globals: Vec<String>,
    pub configure_serial: u32,
}

#[derive(Debug, Default)]
struct TestClientState {
    globals: Vec<String>,
    base_surface: Option<wl_surface::WlSurface>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    xdg_surface: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    configure_serial: Option<u32>,
}

pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug)]
pub struct EnvVarGuard {
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub fn set(name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(name);
        unsafe {
            std::env::set_var(name, value);
        }
        Self { name, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => unsafe {
                std::env::set_var(self.name, previous);
            },
            None => unsafe {
                std::env::remove_var(self.name);
            },
        }
    }
}

#[derive(Debug)]
pub struct RuntimeDirGuard {
    previous: Option<OsString>,
    pub path: PathBuf,
}

impl RuntimeDirGuard {
    pub fn new(prefix: &str) -> Self {
        let path = temporary_runtime_dir(prefix);
        fs::create_dir_all(&path).expect("test runtime dir should be creatable");
        let previous = std::env::var_os("NEKOLAND_RUNTIME_DIR");

        unsafe {
            std::env::set_var("NEKOLAND_RUNTIME_DIR", &path);
        }

        Self { previous, path }
    }
}

impl Drop for RuntimeDirGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => unsafe {
                std::env::set_var("NEKOLAND_RUNTIME_DIR", previous);
            },
            None => unsafe {
                std::env::remove_var("NEKOLAND_RUNTIME_DIR");
            },
        }

        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn run_xdg_client(socket_path: &Path) -> Result<ClientSummary, TestControl> {
    run_xdg_client_with_hold(socket_path, Duration::ZERO)
}

pub fn run_xdg_client_with_hold(
    socket_path: &Path,
    hold_after_configure: Duration,
) -> Result<ClientSummary, TestControl> {
    let stream = UnixStream::connect(socket_path).map_err(classify_io_error)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| TestControl::Fail(format!("set_read_timeout failed: {error}")))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| TestControl::Fail(format!("set_write_timeout failed: {error}")))?;

    let conn = Connection::from_socket(stream)
        .map_err(|error| TestControl::Fail(format!("from_socket failed: {error}")))?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = TestClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while state.configure_serial.is_none() {
        client_dispatch_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(TestControl::Fail(
                "timed out waiting for nekoland to send xdg_surface.configure".to_owned(),
            ));
        }
    }

    event_queue.flush().map_err(classify_wayland_error)?;
    if !hold_after_configure.is_zero() {
        std::thread::sleep(hold_after_configure);
    }

    Ok(ClientSummary {
        globals: state.globals,
        configure_serial: state.configure_serial.ok_or_else(|| {
            TestControl::Fail("client never received xdg_surface.configure".to_owned())
        })?,
    })
}

pub fn assert_globals_present(globals: &[String]) {
    let actual = globals.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "wl_compositor",
        "wl_subcompositor",
        "xdg_wm_base",
        "zxdg_decoration_manager_v1",
        "zwlr_layer_shell_v1",
        "wl_data_device_manager",
        "zwp_primary_selection_device_manager_v1",
        "zwp_linux_dmabuf_v1",
        "wp_viewporter",
        "wp_fractional_scale_manager_v1",
        "wl_shm",
        "wl_seat",
        "wl_output",
        "zxdg_output_manager_v1",
        "wp_presentation",
    ]);

    assert_eq!(actual, expected, "client should observe the full nekoland global registry");
}

#[derive(Debug)]
pub enum TestControl {
    Skip(String),
    Fail(String),
}

fn client_dispatch_once(
    event_queue: &mut EventQueue<TestClientState>,
    state: &mut TestClientState,
) -> Result<(), TestControl> {
    event_queue.dispatch_pending(state).map_err(|error| {
        TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
    })?;
    event_queue.flush().map_err(classify_wayland_error)?;

    let Some(read_guard) = event_queue.prepare_read() else {
        return Ok(());
    };

    read_guard.read().map_err(classify_wayland_error)?;
    event_queue.dispatch_pending(state).map_err(|error| {
        TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
    })?;
    Ok(())
}

fn classify_wayland_error(error: wayland_client::backend::WaylandError) -> TestControl {
    match error {
        wayland_client::backend::WaylandError::Io(error) => classify_io_error(error),
        other => TestControl::Fail(other.to_string()),
    }
}

fn classify_io_error(error: std::io::Error) -> TestControl {
    if matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::TimedOut | ErrorKind::WouldBlock
    ) || error.raw_os_error() == Some(1)
    {
        return TestControl::Skip(error.to_string());
    }

    TestControl::Fail(error.to_string())
}

fn temporary_runtime_dir(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the unix epoch")
        .as_nanos();
    path.push(format!("{prefix}-{}-{unique}", std::process::id()));
    path
}

pub fn default_workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

pub fn write_default_config_with_extra(
    runtime_dir: &Path,
    file_name: &str,
    extra_toml: &str,
) -> PathBuf {
    let mut contents = fs::read_to_string(default_workspace_config_path())
        .expect("default workspace config should be readable in tests");
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    if !extra_toml.trim().is_empty() {
        contents.push('\n');
        contents.push_str(extra_toml.trim());
        contents.push('\n');
    }

    let path = runtime_dir.join(file_name);
    fs::write(&path, contents).expect("temporary test config should be writable");
    path
}

pub fn write_default_config_with_xwayland_disabled(runtime_dir: &Path, file_name: &str) -> PathBuf {
    let source = default_workspace_config_path();
    let mut contents =
        fs::read_to_string(&source).expect("default workspace config should be readable in tests");
    let enabled_block = "[xwayland]\nenabled = true";
    let disabled_block = "[xwayland]\nenabled = false";
    if contents.contains(enabled_block) {
        contents = contents.replacen(enabled_block, disabled_block, 1);
    } else if !contents.contains(disabled_block) {
        if !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push('\n');
        contents.push_str(disabled_block);
        contents.push('\n');
    }

    let path = runtime_dir.join(file_name);
    fs::write(&path, contents).expect("temporary test config should be writable");
    path
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            state.globals.push(interface.clone());

            match interface.as_str() {
                "wl_compositor" => {
                    let compositor =
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ());
                    state.base_surface = Some(compositor.create_surface(qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            state.configure_serial = Some(serial);
            xdg_surface.ack_configure(serial);
            if let Some(surface) = state.base_surface.as_ref() {
                surface.commit();
            }
        }
    }
}

impl TestClientState {
    fn maybe_init_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_none() || self.wm_base.is_none() || self.xdg_surface.is_some() {
            return;
        }

        let surface =
            self.base_surface.as_ref().expect("surface presence checked immediately above");
        let wm_base = self.wm_base.as_ref().expect("wm_base presence checked immediately above");

        let xdg_surface = wm_base.get_xdg_surface(surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.xdg_surface = Some((xdg_surface, toplevel));
    }
}

delegate_noop!(TestClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(TestClientState: ignore wl_surface::WlSurface);
delegate_noop!(TestClientState: ignore xdg_toplevel::XdgToplevel);
