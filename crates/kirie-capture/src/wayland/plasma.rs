//! The windows on KDE Plasma, from `org_kde_plasma_window_management`. KWin offers it only to
//! apps whose desktop entry names it in `X-KDE-Wayland-Interfaces`. Unlike COSMIC, it tells how
//! the windows stack, so their pixels need no comparing.

use wayland_client::globals::registry_queue_init;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_plasma::plasma_virtual_desktop::client::{
    org_kde_plasma_virtual_desktop::{self, OrgKdePlasmaVirtualDesktop},
    org_kde_plasma_virtual_desktop_management::{self, OrgKdePlasmaVirtualDesktopManagement},
};
use wayland_protocols_plasma::plasma_window_management::client::{
    org_kde_plasma_stacking_order::{self, OrgKdePlasmaStackingOrder},
    org_kde_plasma_window::{self, OrgKdePlasmaWindow},
    org_kde_plasma_window_management::{self, OrgKdePlasmaWindowManagement, State as Flag},
};

use super::{State, wait_until};
use crate::CaptureError;
use crate::geometry::Rect;
use crate::windows::Window;

/// The first version that names windows by uuid, as the stacking order does.
const UUIDS: u32 = 13;
/// From this version the stacking order is asked for instead of sent on bind.
const STACKING_ON_REQUEST: u32 = 17;
/// The app id of Plasma's desktop, panels, notifications and on-screen displays.
const PLASMA_SHELL: &str = "org.kde.plasmashell";
/// How long KWin may take to describe its windows before Kirie makes do with those it has.
const DESCRIBING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// What KWin said about one window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PlasmaWindow {
    uuid: String,
    title: String,
    app_id: String,
    rect: Rect,
    /// The `state` flags.
    flags: u32,
    /// The ids of the virtual desktops it is on; none means all of them.
    desktops: Vec<String>,
    /// The ids of the activities it is on; none means all of them.
    activities: Vec<String>,
    unmapped: bool,
    described: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Desktop {
    id: String,
    active: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Listing {
    windows: Vec<PlasmaWindow>,
    /// The windows' uuids, bottom first, once KWin has sent them all.
    stacking: Option<Vec<String>>,
    /// The stacking order as it comes in, before its `done`.
    stacking_so_far: Vec<String>,
    desktops: Vec<Desktop>,
}

impl Listing {
    fn complete(&self) -> bool {
        self.stacking.is_some() && self.windows.iter().all(|window| window.described)
    }
}

/// The windows KWin knows, with how they stack and which virtual desktops are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plasma {
    windows: Vec<PlasmaWindow>,
    stacking: Vec<String>,
    /// The ids of the virtual desktops on screen, if KWin tells.
    active_desktops: Option<Vec<String>>,
}

/// Asks KWin about its windows; `Unsupported` where it does not let Kirie ask, or on other
/// compositors.
pub fn windows() -> Result<Plasma, CaptureError> {
    let connection = Connection::connect_to_env()?;
    let (list, mut queue) = registry_queue_init::<State>(&connection)?;
    let handle = queue.handle();
    let manager: OrgKdePlasmaWindowManagement = list
        .bind(&handle, UUIDS..=18, ())
        .map_err(|_| CaptureError::Unsupported)?;
    let desktops: Option<OrgKdePlasmaVirtualDesktopManagement> = list.bind(&handle, 1..=2, ()).ok();
    if manager.version() >= STACKING_ON_REQUEST {
        manager.get_stacking_order(&handle, ());
    }
    let mut state = State::default();
    // The first brings the windows and desktops, the second what the desktops say on binding.
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;
    if manager.version() < STACKING_ON_REQUEST {
        state.plasma.stacking.get_or_insert_default();
    }
    state.give_up = Some(std::time::Instant::now() + DESCRIBING_TIMEOUT);
    match wait_until(&mut state, &mut queue, |state| state.plasma.complete()) {
        Ok(()) => {}
        Err(CaptureError::Failed(reason)) => {
            tracing::debug!(%reason, "KWin left windows undescribed; taking those it described");
        }
        Err(error) => return Err(error),
    }
    let windows = std::mem::take(&mut state.plasma.windows);
    let stacking = state.plasma.stacking.take().unwrap_or_default();
    let active_desktops = desktops
        .map(|_| {
            state
                .plasma
                .desktops
                .iter()
                .filter(|desktop| desktop.active)
                .map(|desktop| desktop.id.clone())
                .collect::<Vec<_>>()
        })
        .filter(|active| !active.is_empty());
    Ok(Plasma {
        windows,
        stacking,
        active_desktops,
    })
}

impl Plasma {
    /// Whether any window is kept to some activities, so the current one matters.
    pub fn uses_activities(&self) -> bool {
        self.windows
            .iter()
            .any(|window| !window.activities.is_empty())
    }

    /// The windows on screen, topmost first, with `activity` the current one if known.
    pub fn visible(&self, activity: Option<&str>) -> Vec<Window> {
        arrange(
            &self.windows,
            &self.stacking,
            self.active_desktops.as_deref(),
            activity,
        )
    }
}

/// The windows shown on the active desktops and `activity`, topmost first. Windows missing
/// from `stacking` (bottom first) opened after KWin sent it, so they go above the others.
pub(crate) fn arrange(
    windows: &[PlasmaWindow],
    stacking: &[String],
    active_desktops: Option<&[String]>,
    activity: Option<&str>,
) -> Vec<Window> {
    let has = |window: &PlasmaWindow, flag: Flag| window.flags & flag as u32 != 0;
    let on_desktop = |window: &PlasmaWindow| {
        has(window, Flag::OnAllDesktops)
            || window.desktops.is_empty()
            || active_desktops.is_none_or(|active| {
                window
                    .desktops
                    .iter()
                    .any(|desktop| active.contains(desktop))
            })
    };
    let on_activity = |window: &PlasmaWindow| {
        window.activities.is_empty()
            || activity.is_none_or(|current| window.activities.iter().any(|id| id == current))
    };
    let part_of_the_desktop =
        |window: &PlasmaWindow| has(window, Flag::Skiptaskbar) && window.app_id == PLASMA_SHELL;
    let mut found: Vec<(Option<usize>, &PlasmaWindow)> = windows
        .iter()
        .filter(|window| {
            window.described
                && !window.unmapped
                && !window.rect.is_empty()
                && !has(window, Flag::Minimized)
                && !part_of_the_desktop(window)
                && on_desktop(window)
                && on_activity(window)
        })
        .map(|window| {
            (
                stacking.iter().position(|uuid| *uuid == window.uuid),
                window,
            )
        })
        .collect();
    found.sort_by_key(|(height, _)| std::cmp::Reverse(height.unwrap_or(usize::MAX)));
    found
        .into_iter()
        .map(|(_, window)| Window {
            title: window.title.clone(),
            rect: window.rect,
        })
        .collect()
}

impl Dispatch<OrgKdePlasmaWindowManagement, ()> for State {
    fn event(
        state: &mut Self,
        manager: &OrgKdePlasmaWindowManagement,
        event: org_kde_plasma_window_management::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            org_kde_plasma_window_management::Event::WindowWithUuid { uuid, .. } => {
                let index = state.plasma.windows.len();
                manager.get_window_by_uuid(uuid.clone(), handle, index);
                state.plasma.windows.push(PlasmaWindow {
                    uuid,
                    ..PlasmaWindow::default()
                });
            }
            org_kde_plasma_window_management::Event::StackingOrderUuidChanged { uuids } => {
                state.plasma.stacking = Some(
                    uuids
                        .split(';')
                        .filter(|uuid| !uuid.is_empty())
                        .map(str::to_owned)
                        .collect(),
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<OrgKdePlasmaStackingOrder, ()> for State {
    fn event(
        state: &mut Self,
        _: &OrgKdePlasmaStackingOrder,
        event: org_kde_plasma_stacking_order::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            org_kde_plasma_stacking_order::Event::Window { uuid } => {
                state.plasma.stacking_so_far.push(uuid);
            }
            org_kde_plasma_stacking_order::Event::Done => {
                state.plasma.stacking = Some(std::mem::take(&mut state.plasma.stacking_so_far));
            }
            _ => {}
        }
    }
}

impl Dispatch<OrgKdePlasmaWindow, usize> for State {
    fn event(
        state: &mut Self,
        _: &OrgKdePlasmaWindow,
        event: org_kde_plasma_window::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(window) = state.plasma.windows.get_mut(*index) else {
            return;
        };
        match event {
            org_kde_plasma_window::Event::TitleChanged { title } => window.title = title,
            org_kde_plasma_window::Event::AppIdChanged { app_id } => window.app_id = app_id,
            org_kde_plasma_window::Event::StateChanged { flags } => window.flags = flags,
            org_kde_plasma_window::Event::Geometry {
                x,
                y,
                width,
                height,
            } => {
                if let (Ok(width), Ok(height)) = (i32::try_from(width), i32::try_from(height)) {
                    window.rect = Rect::new(x, y, width, height);
                }
            }
            org_kde_plasma_window::Event::VirtualDesktopEntered { id } => {
                window.desktops.push(id);
            }
            org_kde_plasma_window::Event::VirtualDesktopLeft { is } => {
                window.desktops.retain(|desktop| *desktop != is);
            }
            org_kde_plasma_window::Event::ActivityEntered { id } => window.activities.push(id),
            org_kde_plasma_window::Event::ActivityLeft { id } => {
                window.activities.retain(|activity| *activity != id);
            }
            // A window closed since it was announced may get no initial state.
            org_kde_plasma_window::Event::Unmapped => {
                window.unmapped = true;
                window.described = true;
            }
            org_kde_plasma_window::Event::InitialState => window.described = true,
            _ => {}
        }
    }
}

impl Dispatch<OrgKdePlasmaVirtualDesktopManagement, ()> for State {
    fn event(
        state: &mut Self,
        manager: &OrgKdePlasmaVirtualDesktopManagement,
        event: org_kde_plasma_virtual_desktop_management::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let org_kde_plasma_virtual_desktop_management::Event::DesktopCreated {
            desktop_id, ..
        } = event
        {
            let index = state.plasma.desktops.len();
            manager.get_virtual_desktop(desktop_id.clone(), handle, index);
            state.plasma.desktops.push(Desktop {
                id: desktop_id,
                ..Desktop::default()
            });
        }
    }
}

impl Dispatch<OrgKdePlasmaVirtualDesktop, usize> for State {
    fn event(
        state: &mut Self,
        _: &OrgKdePlasmaVirtualDesktop,
        event: org_kde_plasma_virtual_desktop::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(desktop) = state.plasma.desktops.get_mut(*index) else {
            return;
        };
        match event {
            org_kde_plasma_virtual_desktop::Event::Activated => desktop.active = true,
            org_kde_plasma_virtual_desktop::Event::Deactivated
            | org_kde_plasma_virtual_desktop::Event::Removed => desktop.active = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOME: Rect = Rect::new(0, 0, 100, 100);

    fn window(uuid: &str, rect: Rect) -> PlasmaWindow {
        PlasmaWindow {
            uuid: uuid.into(),
            title: uuid.into(),
            rect,
            desktops: vec!["one".into()],
            described: true,
            ..PlasmaWindow::default()
        }
    }

    fn titles(windows: &[Window]) -> Vec<&str> {
        windows.iter().map(|window| window.title.as_str()).collect()
    }

    fn ids(order: &[&str]) -> Vec<String> {
        order.iter().map(|&id| id.to_owned()).collect()
    }

    #[test]
    fn the_stacking_order_puts_the_top_window_first() {
        let windows = [
            window("bottom", SOME),
            window("top", SOME),
            window("middle", SOME),
        ];
        let found = arrange(&windows, &ids(&["bottom", "middle", "top"]), None, None);
        assert_eq!(titles(&found), ["top", "middle", "bottom"]);
    }

    #[test]
    fn windows_opened_after_the_stacking_order_go_on_top() {
        let windows = [window("known", SOME), window("new", SOME)];
        let found = arrange(&windows, &ids(&["known"]), None, None);
        assert_eq!(titles(&found), ["new", "known"]);
    }

    #[test]
    fn geometry_is_already_in_desktop_coordinates() {
        let rect = Rect::new(-1920, 40, 800, 600);
        let found = arrange(&[window("left screen", rect)], &[], None, None);
        assert_eq!(found[0].rect, rect);
    }

    #[test]
    fn only_windows_on_the_active_desktop_show() {
        let elsewhere = PlasmaWindow {
            desktops: vec!["two".into()],
            ..window("elsewhere", SOME)
        };
        let everywhere = PlasmaWindow {
            desktops: Vec::new(),
            ..window("everywhere", SOME)
        };
        let pinned = PlasmaWindow {
            desktops: vec!["two".into()],
            flags: Flag::OnAllDesktops as u32,
            ..window("pinned", SOME)
        };
        let windows = [window("here", SOME), elsewhere, everywhere, pinned];
        let active = ids(&["one"]);
        let found = arrange(&windows, &[], Some(&active), None);
        assert_eq!(titles(&found), ["here", "everywhere", "pinned"]);
    }

    #[test]
    fn without_desktop_information_every_desktop_counts() {
        let elsewhere = PlasmaWindow {
            desktops: vec!["two".into()],
            ..window("elsewhere", SOME)
        };
        assert_eq!(arrange(&[elsewhere], &[], None, None).len(), 1);
    }

    #[test]
    fn only_windows_on_the_current_activity_show() {
        let here = PlasmaWindow {
            activities: vec!["work".into()],
            ..window("here", SOME)
        };
        let elsewhere = PlasmaWindow {
            activities: vec!["home".into()],
            ..window("elsewhere", SOME)
        };
        let windows = [here, elsewhere, window("everywhere", SOME)];
        let found = arrange(&windows, &[], None, Some("work"));
        assert_eq!(titles(&found), ["here", "everywhere"]);
        assert_eq!(arrange(&windows, &[], None, None).len(), 3);
    }

    #[test]
    fn hidden_and_unfinished_windows_are_left_out() {
        let minimized = PlasmaWindow {
            flags: Flag::Minimized as u32,
            ..window("minimized", SOME)
        };
        let unmapped = PlasmaWindow {
            unmapped: true,
            ..window("unmapped", SOME)
        };
        let undescribed = PlasmaWindow {
            described: false,
            ..window("undescribed", SOME)
        };
        let empty = window("empty", Rect::new(0, 0, 0, 50));
        let found = arrange(&[minimized, unmapped, undescribed, empty], &[], None, None);
        assert!(found.is_empty());
    }

    #[test]
    fn the_desktop_and_panels_are_not_windows_to_take() {
        let skips_taskbar = Flag::Skiptaskbar as u32;
        let panel = PlasmaWindow {
            app_id: PLASMA_SHELL.into(),
            flags: skips_taskbar,
            ..window("panel", SOME)
        };
        let drop_down_terminal = PlasmaWindow {
            app_id: "org.kde.yakuake".into(),
            flags: skips_taskbar | Flag::Skipswitcher as u32,
            ..window("drop-down terminal", SOME)
        };
        let settings = PlasmaWindow {
            app_id: PLASMA_SHELL.into(),
            ..window("widget settings", SOME)
        };
        let found = arrange(&[panel, drop_down_terminal, settings], &[], None, None);
        assert_eq!(titles(&found), ["drop-down terminal", "widget settings"]);
    }
}
