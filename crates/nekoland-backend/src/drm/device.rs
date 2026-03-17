use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmNode};
use smithay::backend::session::Session;
use smithay::reexports::drm::control::connector::Interface;
use smithay::reexports::drm::control::{Device as ControlDevice, connector, crtc};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;

use crate::common::outputs::BackendOutputBlueprint;
use crate::traits::BackendDescriptor;

use super::session::{DrmSessionStatus, SharedDrmSessionState};

#[derive(Debug)]
pub struct DrmDeviceState {
    pub node: DrmNode,
    pub path: PathBuf,
    pub device: DrmDevice,
    pub notifier: DrmDeviceNotifier,
    pub fd: DrmDeviceFd,
    pub connectors: Vec<ConnectorInfo>,
}

#[derive(Debug, Clone)]
pub struct ConnectorInfo {
    pub handle: connector::Handle,
    pub local_id: String,
    pub name: String,
    pub connected: bool,
    pub crtc: Option<crtc::Handle>,
    pub properties: OutputProperties,
}

impl ConnectorInfo {
    pub fn output_blueprint(&self, descriptor: &BackendDescriptor) -> BackendOutputBlueprint {
        BackendOutputBlueprint {
            local_id: self.local_id.clone(),
            device: OutputDevice {
                name: self.name.clone(),
                kind: OutputKind::Physical,
                make: "DRM".to_owned(),
                model: descriptor.description.clone(),
            },
            properties: self.properties.clone(),
        }
    }
}

pub type SharedDrmState = Rc<RefCell<Option<DrmDeviceState>>>;

pub fn ensure_drm_device(
    session_state: &SharedDrmSessionState,
    drm_state: &SharedDrmState,
) -> Vec<ConnectorInfo> {
    if drm_state.borrow().is_some() {
        return Vec::new();
    }

    let (mut session, seat_name) = {
        let session_state = session_state.borrow();
        if session_state.status != DrmSessionStatus::Ready || !session_state.active {
            return Vec::new();
        }

        let Some(session) = session_state.session.clone() else {
            tracing::warn!("drm session missing libseat handle");
            return Vec::new();
        };

        (session, session_state.seat_name.clone())
    };

    let Some(path) = find_primary_drm_node(&seat_name) else {
        tracing::warn!("no primary DRM node found; DRM backend unavailable");
        return Vec::new();
    };

    tracing::info!(path = %path.display(), "opening DRM device");

    match open_drm_device(&path, &mut session) {
        Ok(state) => {
            let connected = state
                .connectors
                .iter()
                .filter(|connector| connector.connected)
                .cloned()
                .collect::<Vec<_>>();
            *drm_state.borrow_mut() = Some(state);
            connected
        }
        Err(error) => {
            tracing::warn!(%error, "failed to open DRM device");
            Vec::new()
        }
    }
}

fn find_primary_drm_node(seat_name: &str) -> Option<PathBuf> {
    if let Some(path) = udev_primary_drm_node(seat_name) {
        return Some(path);
    }
    for entry in std::fs::read_dir("/dev/dri").ok()?.flatten() {
        if entry.file_name().to_string_lossy().starts_with("card") {
            return Some(entry.path());
        }
    }
    None
}

fn udev_primary_drm_node(seat_name: &str) -> Option<PathBuf> {
    use smithay::backend::udev::UdevBackend;
    let udev = UdevBackend::new(seat_name).ok()?;
    for (_, path) in udev.device_list() {
        if let Ok(node) = DrmNode::from_path(path)
            && node.ty() == smithay::backend::drm::NodeType::Primary
        {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn open_drm_device(
    path: &std::path::Path,
    session: &mut smithay::backend::session::libseat::LibSeatSession,
) -> Result<DrmDeviceState, Box<dyn std::error::Error>> {
    let owned_fd = session.open(path, OFlags::RDWR | OFlags::CLOEXEC).map_err(|error| {
        std::io::Error::other(format!("failed to open DRM node via libseat: {error}"))
    })?;
    let node = DrmNode::from_path(path)?;
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
            let properties = preferred_output_properties(&info);
            Some(ConnectorInfo {
                handle,
                local_id: connector_local_id(handle),
                name,
                connected,
                crtc,
                properties,
            })
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

fn connector_local_id(handle: connector::Handle) -> String {
    format!("{handle:?}")
}

fn preferred_output_properties(info: &connector::Info) -> OutputProperties {
    let preferred_mode = info
        .modes()
        .iter()
        .find(|mode| {
            mode.mode_type().contains(smithay::reexports::drm::control::ModeTypeFlags::PREFERRED)
        })
        .copied()
        .or_else(|| info.modes().first().copied());

    if let Some(mode) = preferred_mode {
        let (width, height) = mode.size();
        return OutputProperties {
            width: width.max(1) as u32,
            height: height.max(1) as u32,
            refresh_millihz: if mode.vrefresh() > 0 { mode.vrefresh() * 1000 } else { 60_000 },
            scale: 1,
        };
    }

    OutputProperties { width: 2560, height: 1440, refresh_millihz: 144_000, scale: 1 }
}
