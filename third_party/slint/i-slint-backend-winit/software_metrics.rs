// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Opt-in, in-memory measurements for Fastrock's Windows scrolling fixture.
//! No file I/O, logging, or additional redraws on the rendering thread.
use std::cell::RefCell;

#[derive(Clone, Debug)]
pub struct Frame {
    pub render_us: u64,
    pub present_us: u64,
    pub total_us: u64,
    pub dirty_pixels: u64,
    pub window_pixels: u64,
}

thread_local! {
    static FRAMES: RefCell<Option<Vec<Frame>>> = const { RefCell::new(None) };
}

pub fn start() {
    FRAMES.with(|frames| *frames.borrow_mut() = Some(Vec::with_capacity(2048)));
}

pub fn take() -> Vec<Frame> {
    FRAMES.with(|frames| frames.borrow_mut().take().unwrap_or_default())
}

pub(crate) fn enabled() -> bool {
    FRAMES.with(|frames| frames.borrow().is_some())
}

pub(crate) fn record(frame: Frame) {
    FRAMES.with(|frames| {
        if let Some(frames) = frames.borrow_mut().as_mut() {
            // Bound diagnostics even when a fixture fails to finish.
            if frames.len() < 100_000 {
                frames.push(frame);
            }
        }
    });
}
