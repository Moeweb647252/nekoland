use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{NonSendMut, Res, ResMut};
use nekoland_config::resources::CompositorConfig;
use nekoland_core::prelude::AppMetadata;

use crate::manager::SharedBackendManager;
use crate::traits::BackendApplyCtx;

use super::BackendPresentInputs;

/// Let backends consume already-normalized ECS state before presentation.
pub(super) fn backend_apply_system(
    manager: Option<NonSendMut<SharedBackendManager>>,
    app_metadata: Option<Res<AppMetadata>>,
    config: Option<Res<CompositorConfig>>,
    outputs: Res<'_, BackendPresentInputs>,
    winit_window_state: Option<ResMut<'_, crate::winit::backend::WinitWindowState>>,
) -> BevyResult {
    let Some(manager) = manager else {
        return Ok(());
    };
    let mut winit_window_state = winit_window_state;
    let mut ctx = BackendApplyCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &outputs.outputs,
        winit_window_state: winit_window_state.as_deref_mut(),
    };

    manager.borrow_mut().apply_all(&mut ctx).map_err(Into::into)
}
