use anvil_workspace::layout::{DockZone, PaneId, Rect, dock_leaf, dock_zone_for_point};
use anvil_workspace::tab::Tab;

const PANE_CHROME_DRAG_H_BASE: f64 = 30.0;
const PANE_DOCK_DRAG_THRESHOLD_PX: f64 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneDockTarget {
    pub(crate) target: PaneId,
    pub(crate) zone: DockZone,
    pub(crate) rect: Rect,
    pub(crate) preview: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneDockDrag {
    pub(crate) moving: PaneId,
    pub(crate) start_rx: f64,
    pub(crate) start_ry: f64,
    pub(crate) active: bool,
    pub(crate) target: Option<PaneDockTarget>,
}

pub(crate) fn pane_chrome_hit(
    tab: &Tab,
    inner: Rect,
    div_px: f64,
    ui_scale: f64,
    rx: f64,
    ry: f64,
) -> Option<PaneId> {
    let chrome_h = PANE_CHROME_DRAG_H_BASE * ui_scale.max(0.5);
    tab.tree
        .layout(inner, div_px)
        .into_iter()
        .find(|entry| {
            rx >= entry.rect.x
                && rx < entry.rect.x + entry.rect.w
                && ry >= entry.rect.y
                && ry < entry.rect.y + entry.rect.h.min(chrome_h)
        })
        .map(|entry| entry.id)
}

pub(crate) fn dock_preview_rect(rect: Rect, zone: DockZone) -> Rect {
    match zone {
        DockZone::Left => Rect {
            w: rect.w * 0.5,
            ..rect
        },
        DockZone::Right => Rect {
            x: rect.x + rect.w * 0.5,
            w: rect.w * 0.5,
            ..rect
        },
        DockZone::Top => Rect {
            h: rect.h * 0.5,
            ..rect
        },
        DockZone::Bottom => Rect {
            y: rect.y + rect.h * 0.5,
            h: rect.h * 0.5,
            ..rect
        },
        DockZone::Center => rect,
    }
}

pub(crate) fn pane_dock_target(
    tab: &Tab,
    inner: Rect,
    div_px: f64,
    moving: PaneId,
    rx: f64,
    ry: f64,
) -> Option<PaneDockTarget> {
    tab.tree
        .layout(inner, div_px)
        .into_iter()
        .filter(|entry| entry.id != moving)
        .find_map(|entry| {
            let zone = dock_zone_for_point(entry.rect, rx, ry)?;
            if zone == DockZone::Center {
                return None;
            }
            Some(PaneDockTarget {
                target: entry.id,
                zone,
                rect: entry.rect,
                preview: dock_preview_rect(entry.rect, zone),
            })
        })
}

pub(crate) fn update_pane_dock_drag(
    drag: &mut PaneDockDrag,
    tab: &Tab,
    inner: Rect,
    div_px: f64,
    rx: f64,
    ry: f64,
) -> bool {
    let dx = rx - drag.start_rx;
    let dy = ry - drag.start_ry;
    if !drag.active && dx * dx + dy * dy < PANE_DOCK_DRAG_THRESHOLD_PX * PANE_DOCK_DRAG_THRESHOLD_PX
    {
        return false;
    }

    drag.active = true;
    let next = pane_dock_target(tab, inner, div_px, drag.moving, rx, ry);
    if drag.target == next {
        false
    } else {
        drag.target = next;
        true
    }
}

pub(crate) fn finish_pane_dock_drag(tab: &mut Tab, drag: PaneDockDrag) -> bool {
    let Some(target) = drag.target else {
        return false;
    };
    if !drag.active {
        return false;
    }
    dock_leaf(&mut tab.tree, drag.moving, target.target, target.zone).is_ok()
}
