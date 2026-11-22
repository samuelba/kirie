//! The windows on COSMIC's visible workspaces, from `zcosmic_toplevel_info_v1`.
//!
//! COSMIC tells where each window is on each screen and which workspaces are active, but not
//! how windows stack. Where windows overlap, their own pixels tell (see `stacking`); failing
//! that, the focused window is taken as topmost, then smaller windows above larger ones.

use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_output::WlOutput, wl_shm::WlShm};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1,
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};

use super::cosmic::toplevel_info::{
    zcosmic_toplevel_handle_v1::{self, ZcosmicToplevelHandleV1},
    zcosmic_toplevel_info_v1::{self, ZcosmicToplevelInfoV1},
};
use super::{OutputState, State, bind_outputs, ext, wait_until};
use crate::geometry::{Frame, Rect};
use crate::stacking::{self, Matches};
use crate::windows::Window;
use crate::{CaptureError, Cursor};

/// How long capturing the overlapping windows may take; their order is only a refinement.
const STACKING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// What COSMIC said about one window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Toplevel {
    title: String,
    /// Where it is on each screen it shows on: the screen's index, and the place on it.
    placed: Vec<(usize, Rect)>,
    /// The indices of the workspaces it is on.
    workspaces: Vec<usize>,
    minimized: bool,
    activated: bool,
    closed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Listing {
    handles: Vec<ExtForeignToplevelHandleV1>,
    toplevels: Vec<Toplevel>,
    workspaces: Vec<(ExtWorkspaceHandleV1, bool)>,
    /// Whether COSMIC has sent everything about the windows asked for.
    described: bool,
}

/// What COSMIC said about its windows, kept connected to capture them once the screens are in.
#[derive(Debug)]
pub struct Toplevels {
    list: GlobalList,
    queue: EventQueue<State>,
    state: State,
    foreign_list: ExtForeignToplevelListV1,
    workspaces: ExtWorkspaceManagerV1,
    details: Vec<ZcosmicToplevelHandleV1>,
}

/// Asks COSMIC about its windows, which takes it a moment; `Unsupported` on other compositors.
pub fn toplevels() -> Result<Toplevels, CaptureError> {
    let connection = Connection::connect_to_env()?;
    let (list, mut queue) = registry_queue_init::<State>(&connection)?;
    let handle = queue.handle();
    let info: ZcosmicToplevelInfoV1 = list
        .bind(&handle, 3..=3, ())
        .map_err(|_| CaptureError::Unsupported)?;
    let bind_error = |error| CaptureError::Protocol(format!("{error}"));
    let toplevels: ExtForeignToplevelListV1 = list.bind(&handle, 1..=1, ()).map_err(bind_error)?;
    let workspaces: ExtWorkspaceManagerV1 = list.bind(&handle, 1..=1, ()).map_err(bind_error)?;
    let mut state = State::default();
    bind_outputs(&list, &handle, &mut state, &mut queue)?;
    let details = state
        .toplevels
        .handles
        .iter()
        .enumerate()
        .map(|(index, foreign)| info.get_cosmic_toplevel(foreign, &handle, index))
        .collect();
    if !state.toplevels.handles.is_empty() {
        wait_until(&mut state, &mut queue, |state| state.toplevels.described)?;
    }
    Ok(Toplevels {
        list,
        queue,
        state,
        foreign_list: toplevels,
        workspaces,
        details,
    })
}

impl Toplevels {
    /// The windows on the visible workspaces, topmost first, with `frames` the screens as
    /// captured meanwhile.
    pub fn windows(mut self, frames: &[Frame]) -> Vec<Window> {
        let screens: Vec<Rect> = self
            .state
            .outputs
            .iter()
            .map(|output| OutputState::output(output).logical)
            .collect();
        let active: Vec<bool> = self
            .state
            .toplevels
            .workspaces
            .iter()
            .map(|(_, active)| *active)
            .collect();
        let mut visible = arrange(&self.state.toplevels.toplevels, &screens, &active);
        if overlapping(&visible).contains(&true) {
            match self.restack(&visible, frames) {
                Ok(order) => {
                    let mut taken: Vec<Option<(usize, Window)>> =
                        visible.into_iter().map(Some).collect();
                    visible = order.iter().filter_map(|&at| taken[at].take()).collect();
                }
                Err(error) => tracing::debug!(%error, "cannot tell how the windows stack"),
            }
        }
        self.close();
        visible.into_iter().map(|(_, window)| window).collect()
    }

    /// The order of `visible` topmost first, from the pixels of the windows that overlap others.
    fn restack(
        &mut self,
        visible: &[(usize, Window)],
        frames: &[Frame],
    ) -> Result<Vec<usize>, CaptureError> {
        let handle = self.queue.handle();

        let manager: ExtForeignToplevelImageCaptureSourceManagerV1 = self
            .list
            .bind(&handle, 1..=1, ())
            .map_err(|_| CaptureError::Unsupported)?;
        let shm: WlShm = self
            .list
            .bind(&handle, 1..=1, ())
            .map_err(|error| CaptureError::Protocol(format!("no wl_shm: {error}")))?;
        let captured: Vec<usize> = overlapping(visible)
            .iter()
            .enumerate()
            .filter_map(|(at, overlaps)| overlaps.then_some(at))
            .collect();
        let sources = captured
            .iter()
            .map(|&at| {
                manager.create_source(&self.state.toplevels.handles[visible[at].0], &handle, ())
            })
            .collect();
        self.state.give_up = Some(std::time::Instant::now() + STACKING_TIMEOUT);
        let copied = ext::copy(
            &self.list,
            &shm,
            &handle,
            &mut self.state,
            &mut self.queue,
            Cursor::Hidden,
            sources,
        );
        self.state.give_up = None;
        manager.destroy();
        copied?;
        let mut matches: Vec<Option<Matches>> = vec![None; visible.len()];
        for (pending, &at) in self.state.pending.iter().zip(&captured) {
            match pending.view(|view| Matches::new(visible[at].1.rect, view, frames)) {
                Ok(found) => matches[at] = Some(found),
                Err(reason) => {
                    tracing::debug!(%reason, title = %visible[at].1.title, "cannot capture the window");
                }
            }
        }
        self.state.pending.clear();
        Ok(stacking::order(&matches))
    }

    fn close(self) {
        self.details
            .iter()
            .for_each(ZcosmicToplevelHandleV1::destroy);
        self.state
            .toplevels
            .handles
            .iter()
            .for_each(ExtForeignToplevelHandleV1::destroy);
        self.foreign_list.stop();
        self.workspaces.stop();
        if let Err(error) = self.queue.flush() {
            tracing::debug!(%error, "cannot let go of COSMIC's windows");
        }
    }
}

/// For each window, whether it overlaps another.
fn overlapping(visible: &[(usize, Window)]) -> Vec<bool> {
    visible
        .iter()
        .enumerate()
        .map(|(index, (_, window))| {
            visible.iter().enumerate().any(|(other, (_, them))| {
                other != index && window.rect.intersection(&them.rect).is_some()
            })
        })
        .collect()
}

/// The visible windows in desktop coordinates, topmost first as far as COSMIC tells, each with
/// the index of its toplevel.
pub(crate) fn arrange(
    toplevels: &[Toplevel],
    screens: &[Rect],
    active: &[bool],
) -> Vec<(usize, Window)> {
    let visible = |(_, toplevel): &(usize, &Toplevel)| {
        !toplevel.closed
            && !toplevel.minimized
            && (toplevel.workspaces.is_empty()
                || toplevel
                    .workspaces
                    .iter()
                    .any(|&workspace| active.get(workspace).copied().unwrap_or(false)))
    };
    let mut found: Vec<(bool, i64, usize, Window)> = toplevels
        .iter()
        .enumerate()
        .filter(visible)
        .filter_map(|(index, toplevel)| {
            let rect = toplevel
                .placed
                .iter()
                .filter_map(|&(screen, place)| {
                    let screen = screens.get(screen)?;
                    Some(Rect::new(
                        screen.x.saturating_add(place.x),
                        screen.y.saturating_add(place.y),
                        place.width,
                        place.height,
                    ))
                })
                .reduce(|all, rect| all.union(&rect))?;
            (!rect.is_empty()).then(|| {
                (
                    !toplevel.activated,
                    i64::from(rect.width) * i64::from(rect.height),
                    index,
                    Window {
                        title: toplevel.title.clone(),
                        rect,
                    },
                )
            })
        })
        .collect();
    found.sort_by_key(|(behind, area, _, _)| (*behind, *area));
    found
        .into_iter()
        .map(|(_, _, index, window)| (index, window))
        .collect()
}

/// The states in a `state` event: native-endian `u32`s.
fn states(array: &[u8]) -> impl Iterator<Item = u32> + '_ {
    array
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| u32::from_ne_bytes(*bytes))
}

impl Listing {
    fn toplevel(&mut self, handle: &ExtForeignToplevelHandleV1) -> Option<&mut Toplevel> {
        let index = self.handles.iter().position(|known| known == handle)?;
        self.toplevels.get_mut(index)
    }

    fn workspace(&self, handle: &ExtWorkspaceHandleV1) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|(known, _)| known == handle)
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.handles.push(toplevel);
            state.toplevels.toplevels.push(Toplevel::default());
        }
    }

    wayland_client::event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(toplevel) = state.toplevels.toplevel(handle) else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => toplevel.title = title,
            ext_foreign_toplevel_handle_v1::Event::Closed => toplevel.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<ZcosmicToplevelHandleV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let screen_of = |output: &WlOutput, outputs: &[OutputState]| {
            outputs
                .iter()
                .position(|known| known.wl.as_ref() == Some(output))
        };
        match event {
            zcosmic_toplevel_handle_v1::Event::Geometry {
                output,
                x,
                y,
                width,
                height,
            } => {
                let Some(screen) = screen_of(&output, &state.outputs) else {
                    return;
                };
                let Some(toplevel) = state.toplevels.toplevels.get_mut(*index) else {
                    return;
                };
                toplevel.placed.retain(|(known, _)| *known != screen);
                toplevel
                    .placed
                    .push((screen, Rect::new(x, y, width, height)));
            }
            zcosmic_toplevel_handle_v1::Event::State { state: array } => {
                let Some(toplevel) = state.toplevels.toplevels.get_mut(*index) else {
                    return;
                };
                let has = |wanted: zcosmic_toplevel_handle_v1::State| {
                    states(&array).any(|value| value == wanted as u32)
                };
                toplevel.minimized = has(zcosmic_toplevel_handle_v1::State::Minimized);
                toplevel.activated = has(zcosmic_toplevel_handle_v1::State::Activated);
            }
            zcosmic_toplevel_handle_v1::Event::ExtWorkspaceEnter { workspace } => {
                let Some(workspace) = state.toplevels.workspace(&workspace) else {
                    return;
                };
                if let Some(toplevel) = state.toplevels.toplevels.get_mut(*index) {
                    toplevel.workspaces.push(workspace);
                }
            }
            zcosmic_toplevel_handle_v1::Event::ExtWorkspaceLeave { workspace } => {
                let Some(workspace) = state.toplevels.workspace(&workspace) else {
                    return;
                };
                if let Some(toplevel) = state.toplevels.toplevels.get_mut(*index) {
                    toplevel.workspaces.retain(|&known| known != workspace);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_workspace_manager_v1::Event::Workspace { workspace } = event {
            state.toplevels.workspaces.push((workspace, false));
        }
    }

    wayland_client::event_created_child!(State, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_workspace_handle_v1::Event::State {
            state: WEnum::Value(flags),
        } = event
            && let Some(index) = state.toplevels.workspace(handle)
        {
            state.toplevels.workspaces[index].1 =
                flags.contains(ext_workspace_handle_v1::State::Active);
        }
    }
}

impl Dispatch<ZcosmicToplevelInfoV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zcosmic_toplevel_info_v1::Event::Done = event {
            state.toplevels.described = true;
        }
    }
}

delegate_noop!(State: ignore ExtWorkspaceGroupHandleV1);
delegate_noop!(State: ExtForeignToplevelImageCaptureSourceManagerV1);

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: Rect = Rect::new(0, 0, 1920, 1080);
    const RIGHT: Rect = Rect::new(1920, 0, 2560, 1440);

    fn toplevel(title: &str, screen: usize, place: Rect) -> Toplevel {
        Toplevel {
            title: title.into(),
            placed: vec![(screen, place)],
            workspaces: vec![0],
            ..Toplevel::default()
        }
    }

    fn titles(windows: &[(usize, Window)]) -> Vec<&str> {
        windows
            .iter()
            .map(|(_, window)| window.title.as_str())
            .collect()
    }

    #[test]
    fn places_on_a_screen_become_desktop_coordinates() {
        let windows = arrange(
            &[toplevel("editor", 1, Rect::new(10, 20, 800, 600))],
            &[LEFT, RIGHT],
            &[true],
        );
        assert_eq!(windows[0].1.rect, Rect::new(1930, 20, 800, 600));
    }

    #[test]
    fn windows_across_screens_cover_their_part_on_each() {
        let spanning = Toplevel {
            placed: vec![
                (1, Rect::new(0, 100, 300, 400)),
                (0, Rect::new(1720, 100, 200, 400)),
            ],
            ..toplevel("spanning", 0, Rect::default())
        };
        let windows = arrange(&[spanning], &[LEFT, RIGHT], &[true]);
        assert_eq!(windows[0].1.rect, Rect::new(1720, 100, 500, 400));
    }

    #[test]
    fn only_windows_on_active_workspaces_show() {
        let hidden = Toplevel {
            workspaces: vec![1],
            ..toplevel("hidden", 0, Rect::new(0, 0, 100, 100))
        };
        let minimized = Toplevel {
            minimized: true,
            ..toplevel("minimized", 0, Rect::new(0, 0, 100, 100))
        };
        let closed = Toplevel {
            closed: true,
            ..toplevel("closed", 0, Rect::new(0, 0, 100, 100))
        };
        let sticky = Toplevel {
            workspaces: Vec::new(),
            ..toplevel("sticky", 0, Rect::new(0, 0, 100, 100))
        };
        let windows = arrange(
            &[
                toplevel("shown", 0, Rect::new(0, 0, 100, 100)),
                hidden,
                minimized,
                closed,
                sticky,
            ],
            &[LEFT],
            &[true, false],
        );
        assert_eq!(titles(&windows), ["shown", "sticky"]);
    }

    #[test]
    fn the_focused_window_comes_first_then_smaller_ones() {
        let focused = Toplevel {
            activated: true,
            ..toplevel("focused", 0, Rect::new(0, 0, 1920, 1080))
        };
        let windows = arrange(
            &[
                toplevel("big", 0, Rect::new(0, 0, 1000, 1000)),
                focused,
                toplevel("small", 0, Rect::new(0, 0, 200, 200)),
            ],
            &[LEFT],
            &[true],
        );
        assert_eq!(titles(&windows), ["focused", "small", "big"]);
    }

    #[test]
    fn windows_without_a_place_or_size_are_left_out() {
        let unplaced = Toplevel {
            placed: Vec::new(),
            ..toplevel("unplaced", 0, Rect::default())
        };
        let windows = arrange(
            &[
                unplaced,
                toplevel("empty", 0, Rect::new(5, 5, 0, 10)),
                toplevel("unknown screen", 7, Rect::new(0, 0, 10, 10)),
            ],
            &[LEFT],
            &[true],
        );
        assert!(windows.is_empty());
    }

    #[test]
    fn state_arrays_are_native_endian_words() {
        let array: Vec<u8> = [0u32, 2].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(states(&array).collect::<Vec<_>>(), [0, 2]);
    }
}
