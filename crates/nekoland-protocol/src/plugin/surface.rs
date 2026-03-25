//! Surface registry synchronization and platform-surface snapshot extraction.
//!
//! The render and backend layers never touch live Smithay surfaces directly. They consume the
//! normalized snapshot types assembled here from the protocol-owned surface registry.

use smithay::backend::allocator::Buffer;

#[derive(Debug, Clone)]
pub(crate) struct SurfaceIdentity(pub(crate) u64);

#[derive(Debug, Clone, Copy)]
pub(crate) struct XdgSurfaceMarker(pub(crate) nekoland_ecs::resources::XdgSurfaceRole);

#[derive(Debug, Clone, Copy)]
pub(crate) enum InteractiveRequestKind {
    Move,
    Resize,
    PopupGrab,
}

impl InteractiveRequestKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Move => "xdg_toplevel.move",
            Self::Resize => "xdg_toplevel.resize",
            Self::PopupGrab => "xdg_popup.grab",
        }
    }
}

pub(crate) fn sync_protocol_surface_registry_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
    registry: Option<bevy_ecs::prelude::NonSendMut<'_, crate::ProtocolSurfaceRegistry>>,
) {
    let (Some(mut server), Some(mut registry)) = (server, registry) else {
        return;
    };
    server.sync_surface_registry(&mut registry);
}

pub(crate) fn sync_platform_surface_snapshot_state_system(
    registry: Option<bevy_ecs::prelude::NonSend<'_, crate::ProtocolSurfaceRegistry>>,
    dmabuf_support: Option<bevy_ecs::prelude::Res<'_, super::server::ProtocolDmabufSupport>>,
    content_versions: Option<
        bevy_ecs::prelude::Res<'_, nekoland_ecs::resources::SurfaceContentVersionSnapshot>,
    >,
    mut snapshots: bevy_ecs::prelude::ResMut<
        '_,
        nekoland_ecs::resources::PlatformSurfaceSnapshotState,
    >,
) {
    let Some(registry) = registry else {
        return;
    };
    let dmabuf_support = dmabuf_support.as_deref();
    let content_versions = content_versions.as_deref();
    snapshots.surfaces = registry
        .surfaces
        .iter()
        .map(|(surface_id, entry)| {
            let buffer_source = platform_surface_buffer_source(&entry.surface);
            let dmabuf_format = platform_surface_dmabuf_format(&entry.surface);
            (
                *surface_id,
                nekoland_ecs::resources::PlatformSurfaceSnapshot {
                    surface_id: *surface_id,
                    kind: match entry.kind {
                        crate::ProtocolSurfaceKind::Toplevel => {
                            nekoland_ecs::resources::PlatformSurfaceKind::Toplevel
                        }
                        crate::ProtocolSurfaceKind::Popup => {
                            nekoland_ecs::resources::PlatformSurfaceKind::Popup
                        }
                        crate::ProtocolSurfaceKind::Layer => {
                            nekoland_ecs::resources::PlatformSurfaceKind::Layer
                        }
                        crate::ProtocolSurfaceKind::Cursor => {
                            nekoland_ecs::resources::PlatformSurfaceKind::Cursor
                        }
                    },
                    buffer_source,
                    dmabuf_format,
                    import_strategy: platform_surface_import_strategy(
                        buffer_source,
                        dmabuf_format,
                        dmabuf_support,
                    ),
                    attached: platform_surface_attached(&entry.surface),
                    scale: platform_surface_buffer_scale(&entry.surface),
                    content_version: content_versions
                        .and_then(|content_versions| {
                            content_versions.versions.get(surface_id).copied()
                        })
                        .unwrap_or_default(),
                },
            )
        })
        .collect();
}

pub(crate) fn platform_surface_import_strategy(
    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource,
    dmabuf_format: Option<nekoland_ecs::resources::PlatformDmabufFormat>,
    dmabuf_support: Option<&super::server::ProtocolDmabufSupport>,
) -> nekoland_ecs::resources::PlatformSurfaceImportStrategy {
    match buffer_source {
        nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm => {
            nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload
        }
        nekoland_ecs::resources::PlatformSurfaceBufferSource::DmaBuf => {
            let Some(dmabuf_support) = dmabuf_support else {
                return nekoland_ecs::resources::PlatformSurfaceImportStrategy::Unsupported;
            };
            let Some(format) = dmabuf_format.map(platform_dmabuf_format_to_protocol) else {
                return nekoland_ecs::resources::PlatformSurfaceImportStrategy::Unsupported;
            };
            if !dmabuf_support.importable_format(format) {
                nekoland_ecs::resources::PlatformSurfaceImportStrategy::Unsupported
            } else if dmabuf_support.renderable_format(format) {
                nekoland_ecs::resources::PlatformSurfaceImportStrategy::DmaBufImport
            } else {
                nekoland_ecs::resources::PlatformSurfaceImportStrategy::ExternalTextureImport
            }
        }
        nekoland_ecs::resources::PlatformSurfaceBufferSource::SinglePixel => {
            nekoland_ecs::resources::PlatformSurfaceImportStrategy::SinglePixelFill
        }
        nekoland_ecs::resources::PlatformSurfaceBufferSource::Unknown => {
            nekoland_ecs::resources::PlatformSurfaceImportStrategy::Unsupported
        }
    }
}

fn platform_surface_buffer_source(
    surface: &super::WlSurface,
) -> nekoland_ecs::resources::PlatformSurfaceBufferSource {
    super::with_renderer_surface_state(surface, |state| {
        state.buffer().and_then(|buffer| super::buffer_type(buffer)).map(|buffer_type| {
            match buffer_type {
                super::BufferType::Shm => nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                super::BufferType::Dma => {
                    nekoland_ecs::resources::PlatformSurfaceBufferSource::DmaBuf
                }
                super::BufferType::SinglePixel => {
                    nekoland_ecs::resources::PlatformSurfaceBufferSource::SinglePixel
                }
                _ => nekoland_ecs::resources::PlatformSurfaceBufferSource::Unknown,
            }
        })
    })
    .flatten()
    .unwrap_or_default()
}

fn platform_surface_dmabuf_format(
    surface: &super::WlSurface,
) -> Option<nekoland_ecs::resources::PlatformDmabufFormat> {
    super::with_renderer_surface_state(surface, |state| {
        let buffer = state.buffer()?;
        let dmabuf = super::get_dmabuf(buffer).ok()?;
        Some(nekoland_ecs::resources::PlatformDmabufFormat {
            code: dmabuf.format().code as u32,
            modifier: u64::from(dmabuf.format().modifier),
        })
    })
    .flatten()
}

fn platform_dmabuf_format_to_protocol(
    format: nekoland_ecs::resources::PlatformDmabufFormat,
) -> super::DmabufFormat {
    super::DmabufFormat {
        code: smithay::backend::allocator::Fourcc::try_from(format.code)
            .unwrap_or(smithay::backend::allocator::Fourcc::Argb8888),
        modifier: smithay::backend::allocator::Modifier::from(format.modifier),
    }
}

fn platform_surface_attached(surface: &super::WlSurface) -> bool {
    super::compositor::with_states(surface, |states| {
        matches!(
            states.cached_state.get::<super::SurfaceAttributes>().current().buffer,
            Some(super::BufferAssignment::NewBuffer { .. })
        )
    })
}

fn platform_surface_buffer_scale(surface: &super::WlSurface) -> i32 {
    super::compositor::with_states(surface, |states| {
        states.cached_state.get::<super::SurfaceAttributes>().current().buffer_scale
    })
}

impl super::server::ProtocolRuntimeState {
    pub(crate) fn preferred_fractional_scale(&self) -> f64 {
        self.primary_output.current_scale().fractional_scale().max(1.0)
    }

    pub(crate) fn update_surface_fractional_scale(&self, surface: &super::WlSurface) {
        let preferred_scale = self.preferred_fractional_scale();
        super::compositor::with_states(surface, |states| {
            super::with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(preferred_scale);
            });
        });
    }

    pub(crate) fn update_all_fractional_scales(&self) {
        for surface in self.toplevels.values() {
            self.update_surface_fractional_scale(surface.wl_surface());
        }
        for surface in self.popups.values() {
            self.update_surface_fractional_scale(surface.wl_surface());
        }
        for surface in self.layers.values() {
            self.update_surface_fractional_scale(surface.wl_surface());
        }
        for surface in self.x11_windows.values().filter_map(super::X11Surface::wl_surface) {
            self.update_surface_fractional_scale(&surface);
        }
    }

    pub(crate) fn sync_surface_registry(&mut self, registry: &mut crate::ProtocolSurfaceRegistry) {
        registry.surfaces.clear();
        registry.surfaces.extend(self.toplevels.iter().map(|(surface_id, surface)| {
            (
                *surface_id,
                crate::ProtocolSurfaceEntry {
                    kind: crate::ProtocolSurfaceKind::Toplevel,
                    surface: surface.wl_surface().clone(),
                },
            )
        }));
        registry.surfaces.extend(self.popups.iter().map(|(surface_id, surface)| {
            (
                *surface_id,
                crate::ProtocolSurfaceEntry {
                    kind: crate::ProtocolSurfaceKind::Popup,
                    surface: surface.wl_surface().clone(),
                },
            )
        }));
        registry.surfaces.extend(self.layers.iter().map(|(surface_id, surface)| {
            (
                *surface_id,
                crate::ProtocolSurfaceEntry {
                    kind: crate::ProtocolSurfaceKind::Layer,
                    surface: surface.wl_surface().clone(),
                },
            )
        }));
        registry.surfaces.extend(self.x11_window_ids_by_surface.iter().filter_map(
            |(surface_id, window_id)| {
                self.x11_windows.get(window_id).and_then(|window| {
                    window.wl_surface().map(|surface| {
                        let popup = crate::x11_helper_surface(
                            window.is_popup(),
                            super::server::ProtocolRuntimeState::x11_window_type(window),
                        ) && window
                            .is_transient_for()
                            .and_then(|parent_window_id| {
                                self.x11_surface_ids_by_window.get(&parent_window_id).copied()
                            })
                            .is_some();
                        (
                            *surface_id,
                            crate::ProtocolSurfaceEntry {
                                kind: if popup {
                                    crate::ProtocolSurfaceKind::Popup
                                } else {
                                    crate::ProtocolSurfaceKind::Toplevel
                                },
                                surface,
                            },
                        )
                    })
                })
            },
        ));
        if let super::server::ProtocolCursorImage::Surface { surface, .. } =
            &self.cursor_state.image
        {
            let surface = surface.clone();
            let surface_id = self.surface_id(&surface);
            registry.surfaces.insert(
                surface_id,
                crate::ProtocolSurfaceEntry { kind: crate::ProtocolSurfaceKind::Cursor, surface },
            );
        }
    }
}

pub(crate) fn surface_identity(surface: &super::WlSurface, next_surface_id: &mut u64) -> u64 {
    super::compositor::with_states(surface, |states| {
        if let Some(identity) = states.data_map.get::<SurfaceIdentity>() {
            return identity.0;
        }

        let surface_id = *next_surface_id;
        *next_surface_id = next_surface_id.saturating_add(1);
        states.data_map.insert_if_missing_threadsafe(|| SurfaceIdentity(surface_id));
        surface_id
    })
}

pub(crate) fn committed_surface_extent(
    surface: &super::WlSurface,
) -> Option<nekoland_ecs::resources::SurfaceExtent> {
    xdg_window_geometry_extent(surface).or_else(|| {
        super::with_renderer_surface_state(surface, |state| {
            state.surface_size().or_else(|| state.buffer_size())
        })
        .flatten()
        .and_then(surface_extent_from_logical_size)
    })
}

fn renderer_buffer_extent(
    surface: &super::WlSurface,
) -> Option<nekoland_ecs::resources::SurfaceExtent> {
    super::with_renderer_surface_state(surface, |state| state.buffer_size())
        .flatten()
        .and_then(surface_extent_from_logical_size)
}

fn surface_extent_from_logical_size(
    size: smithay::utils::Size<i32, smithay::utils::Logical>,
) -> Option<nekoland_ecs::resources::SurfaceExtent> {
    let width = u32::try_from(size.w).ok()?.max(1);
    let height = u32::try_from(size.h).ok()?.max(1);
    Some(nekoland_ecs::resources::SurfaceExtent { width, height })
}

fn xdg_window_geometry_extent(
    surface: &super::WlSurface,
) -> Option<nekoland_ecs::resources::SurfaceExtent> {
    let geometry = super::compositor::with_states(surface, |states| {
        states
            .cached_state
            .get::<smithay::wayland::shell::xdg::SurfaceCachedState>()
            .current()
            .geometry
    });
    geometry.and_then(xdg_window_geometry_to_extent)
}

fn xdg_window_geometry_to_extent(
    geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
) -> Option<nekoland_ecs::resources::SurfaceExtent> {
    let width = u32::try_from(geometry.size.w).ok()?.max(1);
    let height = u32::try_from(geometry.size.h).ok()?.max(1);
    Some(nekoland_ecs::resources::SurfaceExtent { width, height })
}

fn current_buffer_assignment_extent(
    attributes: &super::SurfaceAttributes,
) -> Option<nekoland_ecs::resources::SurfaceExtent> {
    let super::BufferAssignment::NewBuffer(buffer) = attributes.buffer.as_ref()? else {
        return None;
    };
    let logical_size = smithay::backend::renderer::buffer_dimensions(buffer)?
        .to_logical(attributes.buffer_scale, attributes.buffer_transform.into());
    surface_extent_from_logical_size(logical_size)
}

fn reset_renderer_state_for_resized_buffer(surface: &super::WlSurface) {
    let Some(previous_extent) = renderer_buffer_extent(surface) else {
        return;
    };

    let mut replay_buffer = None;
    let mut replay_damage = Vec::new();
    let mut should_reset = false;

    super::compositor::with_states(surface, |states| {
        let mut attributes = states.cached_state.get::<super::SurfaceAttributes>();
        let current = attributes.current();
        let Some(next_extent) = current_buffer_assignment_extent(current) else {
            return;
        };
        if next_extent == previous_extent {
            return;
        }

        replay_buffer = current.buffer.take();
        replay_damage = std::mem::take(&mut current.damage);
        current.buffer = Some(super::BufferAssignment::Removed);
        should_reset = true;
    });

    if !should_reset {
        return;
    }

    smithay::backend::renderer::utils::on_commit_buffer_handler::<
        super::server::ProtocolRuntimeState,
    >(surface);

    super::compositor::with_states(surface, |states| {
        let mut attributes = states.cached_state.get::<super::SurfaceAttributes>();
        let current = attributes.current();
        current.buffer = replay_buffer.take();
        current.damage = replay_damage;
    });
}

pub(crate) fn layer_cached_state(
    surface: &super::WlSurface,
) -> smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState {
    super::compositor::with_states(surface, |states| {
        let current = *states
            .cached_state
            .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState>()
            .current();
        current
    })
}

pub(crate) fn suggested_layer_surface_size(
    requested_size: smithay::utils::Size<i32, smithay::utils::Logical>,
    output: &smithay::output::Output,
) -> smithay::utils::Size<i32, smithay::utils::Logical> {
    let output_size = output.current_mode().map(|mode| mode.size).unwrap_or((1280, 720).into());
    let width = if requested_size.w > 0 { requested_size.w } else { output_size.w.max(1) };
    let height = if requested_size.h > 0 { requested_size.h } else { output_size.h.max(1) };
    (width.max(1), height.max(1)).into()
}

pub(crate) fn map_layer_level(
    layer: smithay::wayland::shell::wlr_layer::Layer,
) -> nekoland_ecs::components::LayerLevel {
    match layer {
        smithay::wayland::shell::wlr_layer::Layer::Background => {
            nekoland_ecs::components::LayerLevel::Background
        }
        smithay::wayland::shell::wlr_layer::Layer::Bottom => {
            nekoland_ecs::components::LayerLevel::Bottom
        }
        smithay::wayland::shell::wlr_layer::Layer::Top => nekoland_ecs::components::LayerLevel::Top,
        smithay::wayland::shell::wlr_layer::Layer::Overlay => {
            nekoland_ecs::components::LayerLevel::Overlay
        }
    }
}

pub(crate) fn map_layer_anchor(
    anchor: smithay::wayland::shell::wlr_layer::Anchor,
) -> nekoland_ecs::components::LayerAnchor {
    nekoland_ecs::components::LayerAnchor {
        top: anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::TOP),
        bottom: anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::BOTTOM),
        left: anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::LEFT),
        right: anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::RIGHT),
    }
}

pub(crate) fn map_exclusive_zone(
    exclusive_zone: smithay::wayland::shell::wlr_layer::ExclusiveZone,
) -> i32 {
    match exclusive_zone {
        smithay::wayland::shell::wlr_layer::ExclusiveZone::Exclusive(value) => {
            i32::try_from(value).unwrap_or(i32::MAX)
        }
        smithay::wayland::shell::wlr_layer::ExclusiveZone::Neutral => 0,
        smithay::wayland::shell::wlr_layer::ExclusiveZone::DontCare => -1,
    }
}

pub(crate) fn map_layer_margins(
    margins: smithay::wayland::shell::wlr_layer::Margins,
) -> nekoland_ecs::components::LayerMargins {
    nekoland_ecs::components::LayerMargins {
        top: margins.top,
        right: margins.right,
        bottom: margins.bottom,
        left: margins.left,
    }
}

pub(crate) fn mark_xdg_surface(
    surface: &super::WlSurface,
    role: nekoland_ecs::resources::XdgSurfaceRole,
) {
    super::compositor::with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(|| XdgSurfaceMarker(role));
    });
}

pub(crate) fn popup_placement(
    positioner: smithay::wayland::shell::xdg::PositionerState,
    reposition_token: Option<u32>,
) -> nekoland_ecs::resources::PopupPlacement {
    let geometry = positioner.get_geometry();
    nekoland_ecs::resources::PopupPlacement {
        x: geometry.loc.x,
        y: geometry.loc.y,
        width: geometry.size.w,
        height: geometry.size.h,
        reposition_token,
    }
}

pub(crate) fn tracked_xdg_role(
    surface: &super::WlSurface,
) -> Option<nekoland_ecs::resources::XdgSurfaceRole> {
    super::compositor::with_states(surface, |states| {
        states.data_map.get::<XdgSurfaceMarker>().map(|marker| marker.0)
    })
}

pub(crate) fn map_xdg_resize_edge(
    edge: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
) -> nekoland_ecs::resources::ResizeEdges {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge as XdgResizeEdge;

    match edge {
        XdgResizeEdge::Top => nekoland_ecs::resources::ResizeEdges::Top,
        XdgResizeEdge::Bottom => nekoland_ecs::resources::ResizeEdges::Bottom,
        XdgResizeEdge::Left => nekoland_ecs::resources::ResizeEdges::Left,
        XdgResizeEdge::TopLeft => nekoland_ecs::resources::ResizeEdges::TopLeft,
        XdgResizeEdge::BottomLeft => nekoland_ecs::resources::ResizeEdges::BottomLeft,
        XdgResizeEdge::Right => nekoland_ecs::resources::ResizeEdges::Right,
        XdgResizeEdge::TopRight => nekoland_ecs::resources::ResizeEdges::TopRight,
        XdgResizeEdge::BottomRight => nekoland_ecs::resources::ResizeEdges::BottomRight,
        _ => nekoland_ecs::resources::ResizeEdges::BottomRight,
    }
}

impl smithay::wayland::compositor::CompositorHandler for super::server::ProtocolRuntimeState {
    fn compositor_state(&mut self) -> &mut smithay::wayland::compositor::CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a smithay::reexports::wayland_server::Client,
    ) -> &'a smithay::wayland::compositor::CompositorClientState {
        if let Some(client_state) = client.get_data::<super::server::ProtocolClientState>() {
            &client_state.compositor_state
        } else if let Some(client_state) =
            client.get_data::<smithay::xwayland::XWaylandClientData>()
        {
            &client_state.compositor_state
        } else {
            panic!("Wayland clients are created with ProtocolClientState or XWaylandClientData");
        }
    }

    fn commit(&mut self, surface: &super::WlSurface) {
        reset_renderer_state_for_resized_buffer(surface);
        smithay::backend::renderer::utils::on_commit_buffer_handler::<
            super::server::ProtocolRuntimeState,
        >(surface);
        let surface_id = self.surface_id(surface);
        self.popup_manager.commit(surface);
        if let Some(role) = tracked_xdg_role(surface) {
            self.queue_event(crate::ProtocolEvent::SurfaceCommitted {
                surface_id,
                role,
                size: committed_surface_extent(surface),
            });
        } else if self.layers.contains_key(&surface_id) {
            let cached_state = layer_cached_state(surface);
            self.queue_event(crate::ProtocolEvent::LayerSurfaceCommitted {
                surface_id,
                size: committed_surface_extent(surface),
                anchor: map_layer_anchor(cached_state.anchor),
                desired_width: u32::try_from(cached_state.size.w.max(0)).unwrap_or_default(),
                desired_height: u32::try_from(cached_state.size.h.max(0)).unwrap_or_default(),
                exclusive_zone: map_exclusive_zone(cached_state.exclusive_zone),
                margins: map_layer_margins(cached_state.margin),
            });
        }
    }

    fn destroyed(&mut self, surface: &super::WlSurface) {
        let Some(role) = tracked_xdg_role(surface) else {
            return;
        };

        let surface_id = self.surface_id(surface);
        match role {
            nekoland_ecs::resources::XdgSurfaceRole::Toplevel => {
                self.toplevels.remove(&surface_id);
            }
            nekoland_ecs::resources::XdgSurfaceRole::Popup => {
                self.popups.remove(&surface_id);
            }
        }
        self.queue_event(crate::ProtocolEvent::SurfaceDestroyed { surface_id, role });
    }
}

#[cfg(test)]
mod tests {
    use smithay::utils::Rectangle;

    use super::xdg_window_geometry_to_extent;

    #[test]
    fn xdg_window_geometry_extent_uses_client_window_geometry_size() {
        let geometry = Rectangle::new((12, 24).into(), (801, 602).into());
        assert_eq!(
            xdg_window_geometry_to_extent(geometry),
            Some(nekoland_ecs::resources::SurfaceExtent { width: 801, height: 602 })
        );
    }
}

impl smithay::wayland::shell::xdg::XdgShellHandler for super::server::ProtocolRuntimeState {
    fn xdg_shell_state(&mut self) -> &mut smithay::wayland::shell::xdg::XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let surface_id = self.surface_id(&wl_surface);
        mark_xdg_surface(&wl_surface, nekoland_ecs::resources::XdgSurfaceRole::Toplevel);
        self.update_surface_fractional_scale(&wl_surface);
        self.toplevels.insert(surface_id, surface.clone());
        surface.send_configure();
        self.queue_event(crate::ProtocolEvent::ConfigureRequested {
            surface_id,
            role: nekoland_ecs::resources::XdgSurfaceRole::Toplevel,
        });
    }

    fn new_popup(
        &mut self,
        surface: smithay::wayland::shell::xdg::PopupSurface,
        positioner: smithay::wayland::shell::xdg::PositionerState,
    ) {
        let wl_surface = surface.wl_surface().clone();
        let surface_id = self.surface_id(&wl_surface);
        let parent_surface_id = surface.get_parent_surface().map(|parent| self.surface_id(&parent));
        let placement = popup_placement(positioner, None);
        let popup_kind = smithay::desktop::PopupKind::from(surface.clone());

        mark_xdg_surface(&wl_surface, nekoland_ecs::resources::XdgSurfaceRole::Popup);
        self.update_surface_fractional_scale(&wl_surface);
        if let Err(error) = self.popup_manager.track_popup(popup_kind) {
            tracing::warn!(
                surface_id,
                error = %error,
                "failed to register popup with Smithay popup manager"
            );
        }
        self.popups.insert(surface_id, surface.clone());
        let _ = surface.send_configure();
        self.queue_event(crate::ProtocolEvent::PopupCreated {
            surface_id,
            parent_surface_id,
            placement,
        });
        self.queue_event(crate::ProtocolEvent::ConfigureRequested {
            surface_id,
            role: nekoland_ecs::resources::XdgSurfaceRole::Popup,
        });
    }

    fn move_request(
        &mut self,
        surface: smithay::wayland::shell::xdg::ToplevelSurface,
        seat: super::WlSeat,
        serial: smithay::utils::Serial,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        if !self.validate_interactive_request(
            &seat,
            serial,
            surface_id,
            InteractiveRequestKind::Move,
        ) {
            return;
        }

        self.queue_event(crate::ProtocolEvent::MoveRequested {
            surface_id,
            seat_name: self.seat.name().to_owned(),
            serial: serial.into(),
        });
    }

    fn resize_request(
        &mut self,
        surface: smithay::wayland::shell::xdg::ToplevelSurface,
        seat: super::WlSeat,
        serial: smithay::utils::Serial,
        edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        if !self.validate_interactive_request(
            &seat,
            serial,
            surface_id,
            InteractiveRequestKind::Resize,
        ) {
            return;
        }

        self.queue_event(crate::ProtocolEvent::ResizeRequested {
            surface_id,
            seat_name: self.seat.name().to_owned(),
            serial: serial.into(),
            edges: map_xdg_resize_edge(edges),
        });
    }

    fn grab(
        &mut self,
        surface: smithay::wayland::shell::xdg::PopupSurface,
        seat: super::WlSeat,
        serial: smithay::utils::Serial,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        let Some(parent_surface) = surface.get_parent_surface() else {
            tracing::warn!(
                request = InteractiveRequestKind::PopupGrab.as_str(),
                surface_id,
                serial = u32::from(serial),
                "rejecting popup grab because the popup has no parent surface"
            );
            return;
        };
        let popup_kind = smithay::desktop::PopupKind::from(surface.clone());
        let root_surface = match smithay::desktop::find_popup_root_surface(&popup_kind) {
            Ok(root_surface) => root_surface,
            Err(error) => {
                tracing::warn!(
                    request = InteractiveRequestKind::PopupGrab.as_str(),
                    surface_id,
                    serial = u32::from(serial),
                    error = %error,
                    "rejecting popup grab because the popup root surface is no longer alive"
                );
                return;
            }
        };
        let parent_surface_id = self.surface_id(&parent_surface);
        let root_surface_id = self.surface_id(&root_surface);
        if !self.validate_interactive_request(
            &seat,
            serial,
            root_surface_id,
            InteractiveRequestKind::PopupGrab,
        ) {
            tracing::trace!(
                surface_id,
                parent_surface_id,
                root_surface_id,
                serial = u32::from(serial),
                "popup grab validation rejected the popup stack root for this request"
            );
            surface.send_popup_done();
            return;
        }

        let popup_grab = match self.popup_manager.grab_popup::<Self>(
            root_surface,
            popup_kind,
            &self.seat,
            serial,
        ) {
            Ok(popup_grab) => popup_grab,
            Err(error) => {
                tracing::warn!(
                    request = InteractiveRequestKind::PopupGrab.as_str(),
                    surface_id,
                    serial = u32::from(serial),
                    error = %error,
                    "popup grab request was denied by Smithay popup manager; falling back to compositor-side popup grab state"
                );
                self.queue_event(crate::ProtocolEvent::PopupGrabRequested {
                    surface_id,
                    seat_name: self.seat.name().to_owned(),
                    serial: serial.into(),
                });
                return;
            }
        };

        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_grab(self, smithay::desktop::PopupKeyboardGrab::new(&popup_grab), serial);
            keyboard.set_focus(self, Some(surface.wl_surface().clone()), serial);
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.set_grab(
                self,
                smithay::desktop::PopupPointerGrab::new(&popup_grab),
                serial,
                smithay::input::pointer::Focus::Keep,
            );
        }

        self.queue_event(crate::ProtocolEvent::PopupGrabRequested {
            surface_id,
            seat_name: self.seat.name().to_owned(),
            serial: serial.into(),
        });
    }

    fn maximize_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        surface.send_configure();
        self.queue_event(crate::ProtocolEvent::MaximizeRequested { surface_id });
    }

    fn unmaximize_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.queue_event(crate::ProtocolEvent::UnMaximizeRequested { surface_id });
    }

    fn fullscreen_request(
        &mut self,
        surface: smithay::wayland::shell::xdg::ToplevelSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        surface.send_configure();
        self.queue_event(crate::ProtocolEvent::FullscreenRequested {
            surface_id,
            output_name: output.as_ref().map(|output| {
                self.bound_output_names
                    .get(&super::server::wl_output_resource_key(output))
                    .cloned()
                    .unwrap_or_else(|| self.mapped_primary_output_name.clone())
            }),
        });
    }

    fn unfullscreen_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.queue_event(crate::ProtocolEvent::UnFullscreenRequested { surface_id });
    }

    fn minimize_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.queue_event(crate::ProtocolEvent::MinimizeRequested { surface_id });
    }

    fn ack_configure(
        &mut self,
        surface: super::WlSurface,
        configure: smithay::wayland::shell::xdg::Configure,
    ) {
        let surface_id = self.surface_id(&surface);
        let (role, serial) = match configure {
            smithay::wayland::shell::xdg::Configure::Toplevel(configure) => {
                (nekoland_ecs::resources::XdgSurfaceRole::Toplevel, configure.serial.into())
            }
            smithay::wayland::shell::xdg::Configure::Popup(configure) => {
                (nekoland_ecs::resources::XdgSurfaceRole::Popup, configure.serial.into())
            }
        };

        self.queue_event(crate::ProtocolEvent::AckConfigure { surface_id, role, serial });
    }

    fn reposition_request(
        &mut self,
        surface: smithay::wayland::shell::xdg::PopupSurface,
        positioner: smithay::wayland::shell::xdg::PositionerState,
        token: u32,
    ) {
        surface.send_repositioned(token);
        if surface.send_configure().is_ok() {
            let surface_id = self.surface_id(surface.wl_surface());
            self.queue_event(crate::ProtocolEvent::PopupRepositionRequested {
                surface_id,
                placement: popup_placement(positioner, Some(token)),
            });
            self.queue_event(crate::ProtocolEvent::ConfigureRequested {
                surface_id,
                role: nekoland_ecs::resources::XdgSurfaceRole::Popup,
            });
        }
    }

    fn popup_destroyed(&mut self, surface: smithay::wayland::shell::xdg::PopupSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.popups.remove(&surface_id);
        self.queue_event(crate::ProtocolEvent::SurfaceDestroyed {
            surface_id,
            role: nekoland_ecs::resources::XdgSurfaceRole::Popup,
        });
    }

    fn app_id_changed(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        self.queue_toplevel_metadata_changed(&surface);
    }

    fn title_changed(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        self.queue_toplevel_metadata_changed(&surface);
    }
}

impl smithay::wayland::shell::wlr_layer::WlrLayerShellHandler
    for super::server::ProtocolRuntimeState
{
    fn shell_state(&mut self) -> &mut smithay::wayland::shell::wlr_layer::WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: smithay::wayland::shell::wlr_layer::LayerSurface,
        output: Option<super::WlOutput>,
        layer: smithay::wayland::shell::wlr_layer::Layer,
        namespace: String,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.update_surface_fractional_scale(surface.wl_surface());
        let cached_state = layer_cached_state(surface.wl_surface());
        let suggested_size = suggested_layer_surface_size(cached_state.size, &self.primary_output);
        surface.with_pending_state(|state| {
            state.size = Some(suggested_size);
        });
        surface.send_configure();

        self.layers.insert(surface_id, surface);
        self.queue_event(crate::ProtocolEvent::LayerSurfaceCreated {
            surface_id,
            namespace,
            output_name: output.map(|_| self.mapped_primary_output_name.clone()),
            layer: map_layer_level(layer),
            anchor: map_layer_anchor(cached_state.anchor),
            desired_width: u32::try_from(cached_state.size.w.max(0)).unwrap_or_default(),
            desired_height: u32::try_from(cached_state.size.h.max(0)).unwrap_or_default(),
            exclusive_zone: map_exclusive_zone(cached_state.exclusive_zone),
            margins: map_layer_margins(cached_state.margin),
        });
    }

    fn layer_destroyed(&mut self, surface: smithay::wayland::shell::wlr_layer::LayerSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.layers.remove(&surface_id);
        self.queue_event(crate::ProtocolEvent::LayerSurfaceDestroyed { surface_id });
    }
}

impl smithay::wayland::xdg_activation::XdgActivationHandler
    for super::server::ProtocolRuntimeState
{
    fn activation_state(&mut self) -> &mut smithay::wayland::xdg_activation::XdgActivationState {
        &mut self._xdg_activation_state
    }

    fn request_activation(
        &mut self,
        token: smithay::wayland::xdg_activation::XdgActivationToken,
        token_data: smithay::wayland::xdg_activation::XdgActivationTokenData,
        surface: super::WlSurface,
    ) {
        let _ = self.activation_state().remove_token(&token);

        if token_data.timestamp.elapsed() > std::time::Duration::from_secs(10) {
            tracing::warn!(
                token = token.as_str(),
                "ignoring stale xdg_activation request older than compositor policy window"
            );
            return;
        }

        let Some(surface_id) = self.known_surface_id(&surface) else {
            tracing::warn!(
                token = token.as_str(),
                "ignoring xdg_activation request for an unknown surface"
            );
            return;
        };

        if !self.toplevels.contains_key(&surface_id) {
            tracing::warn!(
                token = token.as_str(),
                surface_id,
                "ignoring xdg_activation request for a non-toplevel surface"
            );
            return;
        }

        self.queue_event(crate::ProtocolEvent::ActivationRequested { surface_id });
    }
}

impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler
    for super::server::ProtocolRuntimeState
{
    fn new_decoration(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(
                smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ClientSide,
            );
        });
        toplevel.send_configure();
    }

    fn request_mode(
        &mut self,
        toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
        _mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(
                smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ClientSide,
            );
        });
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(
                smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ClientSide,
            );
        });
        toplevel.send_configure();
    }
}
