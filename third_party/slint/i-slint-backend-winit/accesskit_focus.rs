// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use accesskit::NodeId;

pub(crate) fn published_focus(
    requested: NodeId,
    root: NodeId,
    mut nodes: impl Iterator<Item = NodeId>,
) -> NodeId {
    // An item identity can be allocated by focus evaluation before the adapter
    // has published that item. AccessKit panics on an unknown focus, even when
    // the node list is empty. The pending structural rebuild restores focus.
    if nodes.any(|id| id == requested) {
        requested
    } else {
        root
    }
}
