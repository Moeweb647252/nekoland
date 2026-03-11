use std::cell::RefCell;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::path::PathBuf;
use std::rc::Rc;

use bevy_ecs::prelude::{NonSendMut, Res, ResMut};
use nekoland_ecs::resources::{OutputEventRecord, PendingOutputEvents};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmNode};
use smithay::reexports::drm::control::connector::Interface;
use smithay::reexports::drm::control::{Device as ControlDevice, connector, crtc};
use smithay::utils::DeviceFd;

use crate::traits::{BackendKind, SelectedBackend};

/// All opened DRM device state held across frames.
#[derive(Debug)]
pub struct DrmDeviceState {
    /// The opened DRM node.
    pub node: DrmNode,
    /// Path of the DRM node file.
    pub path: PathBuf,
    /// Smithay DRM device.
    pub device: DrmDevice,
    /// DRM notifier (calloop event source for page-flip/vblank events).
    pub notifier: DrmDeviceNotifier,
    /// DRM file descriptor — used for GBM and connector queries.
    pub fd: DrmDeviceFd,
    /// All connectors discovered at initialisation.
    pub connectors: Vec<ConnectorInfo>,
}

/// Minimal information about a physical display connector.
#[derive(Debug, Clone)]
pub struct ConnectorInfo {
    pub handle: connector::Handle,
    pub name: String,
    pub connected: bool,
    pub crtc: Option<crtc::Handle>,
}

/// Shared reference used between the calloop source and ECS systems.
pub type SharedDrmState = Rc<RefCell<Option<DrmDeviceState>>>;

/// ECS system: open the DRM device on first frame when DRM backend is selected.
pub fn drm_device_system(
    selected_backend: Res<SelectedBackend>,
    drm_state: NonSendMut<SharedDrmState>,
    mut pending_output_events: ResMut<PendingOutputEvents>,
) {
    if selected_backend.kind != BackendKind::Drm {
        return;
    }

    if drm_state.borrow().is_some() {
        return;
    }

    let Some(path) = find_primary_drm_node() else {
        tracing::warn!("no primary DRM node found; DRM backend unavailable");
        return;
    };

    tracing::info!(path = %path.display(), "opening DRM device");

    match open_drm_device(&path) {
        Ok(state) => {
            for connector in &state.connectors {
                if connector.connected {
                    tracing::info!(connector = %connector.name, "DRM connector connected");
                    pending_output_events.items.push(OutputEventRecord {
                        output_name: connector.name.clone(),
                        change: "connected".to_owned(),
                    });
                }
            }
            *drm_state.borrow_mut() = Some(state);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to open DRM device");
        }
    }
}

/// Scan `/dev/dri/` for a primary DRM node.
fn find_primary_drm_node() -> Option<PathBuf> {
    if let Some(path) = udev_primary_drm_node() {
        return Some(path);
    }
    for entry in std::fs::read_dir("/dev/dri").ok()?.flatten() {
        if entry.file_name().to_string_lossy().starts_with("card") {
            return Some(entry.path());
        }
    }
    None
}

/// Try to locate the primary GPU via udev.
fn udev_primary_drm_node() -> Option<PathBuf> {
    use smithay::backend::udev::UdevBackend;
    let udev = UdevBackend::new("seat0").ok()?;
    for (_, path) in udev.device_list() {
        if let Ok(node) = DrmNode::from_path(&path) {
            if node.ty() == smithay::backend::drm::NodeType::Primary {
                return Some(path.to_path_buf());
            }
        }
    }
    None
}

/// Open a DRM device and enumerate connectors.
fn open_drm_device(path: &std::path::Path) -> Result<DrmDeviceState, Box<dyn std::error::Error>> {
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;

    let node = DrmNode::from_path(path)?;
    // SAFETY: the file was just opened and the fd is valid.
    let owned_fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(file.into_raw_fd()) };
    let device_fd = DrmDeviceFd::new(DeviceFd::from(owned_fd));

    let (device, notifier) = DrmDevice::new(device_fd.clone(), false)?;

    let resources = device_fd.resource_handles()?;
    let connectors = resources
        .connectors()
        .iter()
        .filter_map(|&handle| {
            let info = device_fd.get_connector(handle, false).ok()?;
            let name = connector_name(&info);
            let connected =
                info.state() == smithay::reexports::drm::control::connector::State::Connected;
            let crtc = info
                .current_encoder()
                .and_then(|enc| device_fd.get_encoder(enc).ok())
                .and_then(|enc| enc.crtc());
            Some(ConnectorInfo { handle, name, connected, crtc })
        })
        .collect();

    Ok(DrmDeviceState {
        node,
        path: path.to_path_buf(),
        device,
        notifier,
        fd: device_fd,
        connectors,
    })
}

/// Build a human-readable connector name (e.g. `"HDMI-A-1"`).
fn connector_name(info: &connector::Info) -> String {
    let interface = match info.interface() {
        Interface::DVII => "DVI-I",
        Interface::DVID => "DVI-D",
        Interface::DVIA => "DVI-A",
        Interface::Composite => "Composite",
        Interface::SVideo => "SVIDEO",
        Interface::LVDS => "LVDS",
        Interface::Component => "Component",
        Interface::NinePinDIN => "DIN",
        Interface::DisplayPort => "DP",
        Interface::HDMIA => "HDMI-A",
        Interface::HDMIB => "HDMI-B",
        Interface::TV => "TV",
        Interface::EmbeddedDisplayPort => "eDP",
        Interface::Virtual => "Virtual",
        Interface::DSI => "DSI",
        _ => "Unknown",
    };
    format!("{}-{}", interface, info.interface_id())
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use nekoland_ecs::resources::PendingOutputEvents;

    use crate::traits::{BackendKind, SelectedBackend};

    use super::SharedDrmState;

    #[test]
    fn drm_device_system_is_noop_when_backend_is_not_drm() {
        let mut world = World::new();
        world.insert_resource(SelectedBackend {
            kind: BackendKind::Winit,
            description: String::new(),
        });
        world.insert_resource(PendingOutputEvents::default());
        world.insert_non_send_resource(SharedDrmState::default());

        let selected: &SelectedBackend = world.resource();
        assert_ne!(selected.kind, BackendKind::Drm);

        let events: &PendingOutputEvents = world.resource();
        assert!(events.items.is_empty());

        let drm: &SharedDrmState = world.non_send_resource();
        assert!(drm.borrow().is_none());
    }
}
