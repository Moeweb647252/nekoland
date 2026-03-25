//! Screenshot request queues and completed readback frame storage.

#![allow(missing_docs)]

use std::collections::VecDeque;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Stable identity for one internal screenshot/readback request.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct ScreenshotRequestId(pub u64);

/// One pending request to read back the final presented pixels for an output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputScreenshotRequest {
    pub id: ScreenshotRequestId,
    pub output_id: OutputId,
}

/// Internal queue of pending screenshot/readback requests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingScreenshotRequests {
    next_id: u64,
    pub requests: Vec<OutputScreenshotRequest>,
}

impl PendingScreenshotRequests {
    /// Enqueues one screenshot request for the given output and returns its stable id.
    pub fn request_output(&mut self, output_id: OutputId) -> ScreenshotRequestId {
        let id = ScreenshotRequestId(self.next_id.max(1));
        self.next_id = id.0.saturating_add(1);
        self.requests.push(OutputScreenshotRequest { id, output_id });
        id
    }

    /// Returns pending screenshot requests targeting one output.
    pub fn requests_for_output(&self, output_id: OutputId) -> Vec<OutputScreenshotRequest> {
        self.requests.iter().filter(|request| request.output_id == output_id).cloned().collect()
    }

    /// Returns pending requests whose ids are present in the provided slice.
    pub fn requests_by_ids(
        &self,
        request_ids: &[ScreenshotRequestId],
    ) -> Vec<OutputScreenshotRequest> {
        if request_ids.is_empty() {
            return Vec::new();
        }

        self.requests.iter().filter(|request| request_ids.contains(&request.id)).cloned().collect()
    }

    /// Removes completed requests from the pending queue.
    pub fn finish_requests(&mut self, completed_ids: &[ScreenshotRequestId]) {
        if completed_ids.is_empty() {
            return;
        }

        self.requests.retain(|request| !completed_ids.contains(&request.id));
    }
}

/// One completed output-local screenshot/readback frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenshotFrame {
    pub request_id: ScreenshotRequestId,
    pub output_id: OutputId,
    pub frame: u64,
    pub uptime_millis: u64,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub pixels_rgba: Vec<u8>,
}

/// Ring buffer of completed screenshot/readback frames.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletedScreenshotFrames {
    pub frame_limit: usize,
    pub frames: VecDeque<ScreenshotFrame>,
}

impl Default for CompletedScreenshotFrames {
    fn default() -> Self {
        Self { frame_limit: 4, frames: VecDeque::new() }
    }
}

impl CompletedScreenshotFrames {
    /// Pushes one completed screenshot frame, trimming the ring buffer to its configured limit.
    pub fn push_frame(&mut self, frame: ScreenshotFrame) {
        self.frames.push_back(frame);
        while self.frames.len() > self.frame_limit.max(1) {
            self.frames.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;

    use super::{CompletedScreenshotFrames, PendingScreenshotRequests, ScreenshotFrame};

    #[test]
    fn pending_requests_allocate_stable_ids_and_filter_by_output() {
        let mut requests = PendingScreenshotRequests::default();

        let first = requests.request_output(OutputId(7));
        let second = requests.request_output(OutputId(9));

        assert_eq!(first.0, 1);
        assert_eq!(second.0, 2);
        assert_eq!(requests.requests_for_output(OutputId(7)).len(), 1);

        requests.finish_requests(&[first]);
        assert!(requests.requests_for_output(OutputId(7)).is_empty());
        assert_eq!(requests.requests_for_output(OutputId(9)).len(), 1);
    }

    #[test]
    fn completed_frames_trim_to_limit() {
        let mut frames = CompletedScreenshotFrames { frame_limit: 2, ..Default::default() };
        frames.push_frame(ScreenshotFrame {
            request_id: super::ScreenshotRequestId(1),
            ..Default::default()
        });
        frames.push_frame(ScreenshotFrame {
            request_id: super::ScreenshotRequestId(2),
            ..Default::default()
        });
        frames.push_frame(ScreenshotFrame {
            request_id: super::ScreenshotRequestId(3),
            ..Default::default()
        });

        assert_eq!(frames.frames.len(), 2);
        assert_eq!(frames.frames.front().map(|frame| frame.request_id.0), Some(2));
        assert_eq!(frames.frames.back().map(|frame| frame.request_id.0), Some(3));
    }
}
