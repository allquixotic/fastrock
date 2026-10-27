// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Private raster storage independent of Windows/RDP's presentation-buffer age.
#[derive(Default)]
pub struct RetainedBuffer<P> {
    size: (u32, u32),
    pixels: Vec<P>,
}

impl<P: Default + Clone> RetainedBuffer<P> {
    /// Returns storage and whether the renderer must repaint every pixel.
    /// Compare dimensions, not just area: equal-area resize changes row stride.
    pub fn prepare(&mut self, width: u32, height: u32) -> (&mut [P], bool) {
        let changed = self.size != (width, height);
        if changed {
            self.size = (width, height);
            self.pixels.resize(width as usize * height as usize, P::default());
        }
        (&mut self.pixels, changed)
    }

    /// Repair all presentation pixels even if no scene pixels were dirty.
    pub fn present_to(&self, target: &mut [P]) {
        target.clone_from_slice(&self.pixels);
    }
}
