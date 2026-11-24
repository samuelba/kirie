//! Where the windows on screen are, so the overlay can pick one. Wayland gives clients no way to
//! ask, so this asks whoever knows: Kirie's GNOME Shell extension, COSMIC, KDE Plasma, sway or
//! Hyprland.

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::geometry::Rect;

/// A window in desktop coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub title: String,
    pub rect: Rect,
}

/// Finding the windows, begun with [`start`] while the screens are captured.
#[derive(Debug)]
pub enum Lookup {
    /// Kirie's GNOME Shell extension, asked once the capture is done.
    Shell,
    Cosmic(Box<crate::wayland::Toplevels>),
    /// KWin's windows, before Plasma's activity manager tells which ones the current activity
    /// shows.
    Plasma(crate::wayland::Plasma),
    Found(Vec<Window>),
}

/// Starts finding the windows on the visible workspaces. It blocks, so run it beside the capture.
pub fn start() -> Lookup {
    if crate::shell::launched() {
        // The extension's Wayland connection is the overlay's; no other one is made.
        return Lookup::Shell;
    }
    let found = if let Some(socket) = std::env::var_os("SWAYSOCK") {
        sway(&PathBuf::from(socket))
    } else if let Ok(signature) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        hyprland(&signature)
    } else {
        match crate::wayland::toplevels() {
            Ok(toplevels) => return Lookup::Cosmic(Box::new(toplevels)),
            Err(crate::CaptureError::Unsupported) => match crate::wayland::plasma_windows() {
                Ok(plasma) => return Lookup::Plasma(plasma),
                Err(crate::CaptureError::Unsupported) => Ok(Vec::new()),
                Err(error) => Err(std::io::Error::other(error)),
            },
            Err(error) => Err(std::io::Error::other(error)),
        }
    };
    Lookup::Found(
        found
            .inspect_err(|error| tracing::debug!(%error, "cannot list the windows"))
            .unwrap_or_default(),
    )
}

impl Lookup {
    /// The windows, topmost first, with `frames` the screens as captured meanwhile; empty
    /// where nobody can tell.
    pub async fn finish(self, frames: &[crate::Frame]) -> Vec<Window> {
        match self {
            Lookup::Shell => crate::shell::windows().await.unwrap_or_default(),
            Lookup::Cosmic(toplevels) => toplevels.windows(frames),
            Lookup::Plasma(plasma) => {
                let activity = if plasma.uses_activities() {
                    crate::activities::current().await
                } else {
                    None
                };
                plasma.visible(activity.as_deref())
            }
            Lookup::Found(windows) => windows,
        }
    }
}

fn connect(path: &std::path::Path) -> std::io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(stream)
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

/// sway's IPC: `GET_TREE`, then every visible window in it.
fn sway(socket: &std::path::Path) -> std::io::Result<Vec<Window>> {
    const GET_TREE: u32 = 4;
    let mut stream = connect(socket)?;
    let mut request = b"i3-ipc".to_vec();
    request.extend_from_slice(&0u32.to_ne_bytes());
    request.extend_from_slice(&GET_TREE.to_ne_bytes());
    stream.write_all(&request)?;
    let mut header = [0u8; 14];
    stream.read_exact(&mut header)?;
    if &header[..6] != b"i3-ipc" {
        return Err(invalid("not a sway IPC reply"));
    }
    let length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]) as usize;
    if length > 64 * 1024 * 1024 {
        return Err(invalid("the sway tree is too large"));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    let tree: Value = serde_json::from_slice(&body).map_err(|error| invalid(error.to_string()))?;
    Ok(sway_windows(&tree))
}

/// Visible windows of a sway tree, floating ones first as they sit on top.
pub fn sway_windows(tree: &Value) -> Vec<Window> {
    let mut floating = Vec::new();
    let mut tiled = Vec::new();
    collect(tree, false, &mut floating, &mut tiled);
    floating.into_iter().chain(tiled).collect()
}

fn collect(node: &Value, in_floating: bool, floating: &mut Vec<Window>, tiled: &mut Vec<Window>) {
    let visible = node["visible"].as_bool().unwrap_or(false);
    let is_window = node["pid"].is_number() || node["window"].is_number();
    if is_window
        && visible
        && let Some(rect) = rect_of(&node["rect"])
    {
        let window = Window {
            title: node["name"].as_str().unwrap_or_default().to_owned(),
            rect,
        };
        if in_floating {
            floating.push(window);
        } else {
            tiled.push(window);
        }
    }
    for child in node["nodes"].as_array().into_iter().flatten() {
        collect(child, in_floating, floating, tiled);
    }
    for child in node["floating_nodes"].as_array().into_iter().flatten() {
        collect(child, true, floating, tiled);
    }
}

fn rect_of(value: &Value) -> Option<Rect> {
    let number = |key: &str| value[key].as_i64().and_then(|n| i32::try_from(n).ok());
    let rect = Rect::new(
        number("x")?,
        number("y")?,
        number("width")?,
        number("height")?,
    );
    (!rect.is_empty()).then_some(rect)
}

/// Hyprland's socket: the clients on each monitor's active workspace.
fn hyprland(signature: &str) -> std::io::Result<Vec<Window>> {
    let ask = |command: &str| -> std::io::Result<Value> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let candidates = runtime
            .map(|dir| dir.join("hypr").join(signature).join(".socket.sock"))
            .into_iter()
            .chain([PathBuf::from(format!("/tmp/hypr/{signature}/.socket.sock"))]);
        let mut last = invalid("no Hyprland socket");
        for path in candidates {
            match connect(&path) {
                Ok(mut stream) => {
                    stream.write_all(command.as_bytes())?;
                    let mut reply = Vec::new();
                    stream.read_to_end(&mut reply)?;
                    return serde_json::from_slice(&reply)
                        .map_err(|error| invalid(error.to_string()));
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    };
    Ok(hyprland_windows(&ask("j/monitors")?, &ask("j/clients")?))
}

/// The clients on an active workspace, most recently focused first.
pub fn hyprland_windows(monitors: &Value, clients: &Value) -> Vec<Window> {
    let active: Vec<i64> = monitors
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|monitor| monitor["activeWorkspace"]["id"].as_i64())
        .collect();
    let mut found: Vec<(i64, Window)> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|client| {
            client["mapped"].as_bool().unwrap_or(true)
                && !client["hidden"].as_bool().unwrap_or(false)
                && client["workspace"]["id"]
                    .as_i64()
                    .is_some_and(|id| active.contains(&id))
        })
        .filter_map(|client| {
            let pair = |key: &str| {
                let values = client[key].as_array()?;
                Some((
                    i32::try_from(values.first()?.as_i64()?).ok()?,
                    i32::try_from(values.get(1)?.as_i64()?).ok()?,
                ))
            };
            let ((x, y), (width, height)) = (pair("at")?, pair("size")?);
            Some((
                client["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                Window {
                    title: client["title"].as_str().unwrap_or_default().to_owned(),
                    rect: Rect::new(x, y, width, height),
                },
            ))
        })
        .collect();
    found.sort_by_key(|(order, _)| *order);
    found.into_iter().map(|(_, window)| window).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sway_trees_give_visible_windows_floating_first() {
        let tree = json!({
            "nodes": [{
                "nodes": [
                    {"pid": 1, "visible": true, "name": "editor", "rect": {"x": 0, "y": 0, "width": 800, "height": 600}},
                    {"pid": 2, "visible": false, "name": "hidden", "rect": {"x": 0, "y": 0, "width": 800, "height": 600}}
                ],
                "floating_nodes": [
                    {"pid": 3, "visible": true, "name": "dialog", "rect": {"x": 100, "y": 100, "width": 200, "height": 100}}
                ]
            }]
        });
        let windows = sway_windows(&tree);
        let titles: Vec<&str> = windows.iter().map(|window| window.title.as_str()).collect();
        assert_eq!(titles, ["dialog", "editor"]);
        assert_eq!(windows[0].rect, Rect::new(100, 100, 200, 100));
    }

    #[test]
    fn hyprland_clients_on_active_workspaces_by_focus() {
        let monitors = json!([{"activeWorkspace": {"id": 1}}]);
        let clients = json!([
            {"title": "older", "at": [0, 0], "size": [500, 400], "workspace": {"id": 1}, "focusHistoryID": 2},
            {"title": "newest", "at": [50, 50], "size": [300, 200], "workspace": {"id": 1}, "focusHistoryID": 0},
            {"title": "elsewhere", "at": [0, 0], "size": [10, 10], "workspace": {"id": 2}, "focusHistoryID": 1},
            {"title": "hidden", "at": [0, 0], "size": [10, 10], "workspace": {"id": 1}, "hidden": true}
        ]);
        let windows = hyprland_windows(&monitors, &clients);
        let titles: Vec<&str> = windows.iter().map(|window| window.title.as_str()).collect();
        assert_eq!(titles, ["newest", "older"]);
    }

    #[test]
    fn empty_rects_are_skipped() {
        assert_eq!(
            rect_of(&json!({"x": 0, "y": 0, "width": 0, "height": 5})),
            None
        );
        assert_eq!(rect_of(&json!({"x": 0})), None);
    }
}
