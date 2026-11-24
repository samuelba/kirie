import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const SHORTCUT_KEY = 'capture-shortcut';
const BUS_NAME = 'dev.samuelba.Kirie.Shell';
const OBJECT_PATH = '/dev/samuelba/Kirie/Shell';
const OVERLAY_TITLE = /^kirie-overlay-(\d+)$/;
const INTERFACE = `
<node>
  <interface name="${BUS_NAME}">
    <method name="Capture">
      <arg name="edit" type="b" direction="in"/>
      <arg name="cursor" type="b" direction="in"/>
    </method>
    <method name="Screens">
      <arg name="screens" type="a(uiiiis)" direction="out"/>
    </method>
    <method name="Windows">
      <arg name="windows" type="a(siiii)" direction="out"/>
    </method>
    <method name="SetClipboard">
      <arg name="mime" type="s" direction="in"/>
      <arg name="data" type="ay" direction="in"/>
    </method>
  </interface>
</node>`;

/** GNOME 47 replaced WaylandClient.new() and spawnv() with new_subprocess(). */
function spawnClient(launcher, argv) {
    if (Meta.WaylandClient.new_subprocess) {
        const client = Meta.WaylandClient.new_subprocess(global.context, launcher, argv);
        return {client, subprocess: client.get_subprocess()};
    }
    const client = Meta.WaylandClient.new(global.context, launcher);
    return {client, subprocess: client.spawnv(global.display, argv)};
}

/** At most this many strips per monitor; more stop paying off. */
const MAX_STRIPS = 8;
/** Strip heights are multiples of this many logical pixels. */
const STRIP_STEP = 8;

/** Writes a PNG of `[x, y, width, height]` of the stage to `path`, readable only by this user. */
async function shoot([x, y, width, height], path) {
    const stream = Gio.File.new_for_path(path).replace(
        null, false, Gio.FileCreateFlags.PRIVATE, null);
    try {
        await new Shell.Screenshot().screenshot_area(x, y, width, height, stream);
    } finally {
        stream.close(null);
    }
}

/** The process id behind a D-Bus caller. */
function callerPid(sender) {
    const reply = Gio.DBus.session.call_sync(
        'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
        'GetConnectionUnixProcessID', new GLib.Variant('(s)', [sender]),
        new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, -1, null);
    return reply.deepUnpack()[0];
}

/**
 * Takes screenshots with Kirie: launches it as a trusted Wayland client, hands it the screens
 * without a permission dialog, and keeps its overlay above every window. Only processes this
 * extension started may read the screens, so it never becomes a way around GNOME's screenshot
 * permission.
 */
export default class KirieExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._clients = new Map();
        this._windowCreatedId = global.display.connect('window-created',
            (_display, window) => this._adopt(window));
        Main.wm.addKeybinding(
            SHORTCUT_KEY,
            this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._launch(false, false));
        this._exported = Gio.DBusExportedObject.wrapJSObject(INTERFACE, {
            Capture: (edit, cursor) => this._launch(edit, cursor),
            ScreensAsync: (params, invocation) => this._screens(invocation),
            SetClipboardAsync: (params, invocation) => this._setClipboard(params, invocation),
            WindowsAsync: (params, invocation) => this._windows(invocation),
        });
        this._exported.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
    }

    disable() {
        Main.wm.removeKeybinding(SHORTCUT_KEY);
        Gio.bus_unown_name(this._nameId);
        this._exported.unexport();
        global.display.disconnect(this._windowCreatedId);
        this._clients.clear();
        this._settings = null;
    }

    /**
     * The command that starts Kirie: `kirie` from PATH, or the configured binary, which must be
     * a single absolute path to a file named kirie that only this user can change. Anything else
     * would let whoever can write the setting run a program with the extension's trust.
     */
    _command() {
        const configured = this._settings.get_strv('command');
        if (configured.length === 0)
            return GLib.find_program_in_path('kirie');
        if (configured.length !== 1 || !GLib.path_is_absolute(configured[0]) ||
            GLib.path_get_basename(configured[0]) !== 'kirie')
            return null;
        try {
            const info = Gio.File.new_for_path(configured[0]).query_info(
                'standard::type,unix::uid,unix::mode', Gio.FileQueryInfoFlags.NONE, null);
            const regular = info.get_file_type() === Gio.FileType.REGULAR;
            const owned = info.get_attribute_uint32('unix::uid') === new Gio.Credentials().get_unix_user();
            const private_ = (info.get_attribute_uint32('unix::mode') & 0o022) === 0;
            return regular && owned && private_ ? configured[0] : null;
        } catch {
            return null;
        }
    }

    _launch(edit, cursor) {
        // One capture at a time, so nothing can pile up Kirie processes.
        if (this._clients.size > 0)
            return;
        const program = this._command();
        if (!program) {
            Main.notifyError('Kirie is not installed',
                'The extension cannot find the kirie program. Reinstall Kirie to take screenshots with it.');
            return;
        }
        const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
        launcher.setenv('KIRIE_SHELL_LAUNCHED', '1', true);
        const argv = [program, 'capture', 'region'];
        if (cursor)
            argv.push('--cursor');
        if (edit)
            argv.push('--edit');
        let spawned;
        try {
            spawned = spawnClient(launcher, argv);
        } catch (error) {
            Main.notifyError('Kirie could not start', error.message);
            return;
        }
        const pid = Number(spawned.subprocess.get_identifier());
        this._clients.set(pid, spawned.client);
        spawned.subprocess.wait_async(null, () => this._clients.delete(pid));
    }

    /** Whether the caller is a Kirie this extension started; answers the call if not. */
    _trusted(invocation) {
        let pid;
        try {
            pid = callerPid(invocation.get_sender());
        } catch (error) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', error.message);
            return false;
        }
        if (this._clients.has(pid))
            return true;
        invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied',
            'Only a Kirie started by the extension may do this');
        return false;
    }

    _setClipboard([mime, data], invocation) {
        if (!this._trusted(invocation))
            return;
        if (mime !== 'image/png') {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.InvalidArgs',
                'Only PNG images are copied');
            return;
        }
        St.Clipboard.get_default().set_content(
            St.ClipboardType.CLIPBOARD, mime, new GLib.Bytes(data));
        invocation.return_value(null);
    }

    /** The windows on the active workspace, topmost first, for picking one in the overlay. */
    _windows(invocation) {
        if (!this._trusted(invocation))
            return;
        const workspace = global.workspace_manager.get_active_workspace();
        const windows = global.display.sort_windows_by_stacking(workspace.list_windows())
            .reverse()
            .filter(window => !window.minimized &&
                window.get_window_type() === Meta.WindowType.NORMAL)
            .map(window => {
                const rect = window.get_frame_rect();
                return [window.get_title() ?? '', rect.x, rect.y, rect.width, rect.height];
            });
        invocation.return_value(new GLib.Variant('(a(siiii))', [windows]));
    }

    _screens(invocation) {
        if (!this._trusted(invocation))
            return;
        this._capture().then(screens => {
            invocation.return_value(new GLib.Variant('(a(uiiiis))', [screens]));
        }).catch(error => {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', error.message);
        });
    }

    /**
     * PNGs of every monitor in the user's runtime folder, each with the area it shows. A monitor
     * comes in horizontal strips, `shell-<monitor>-<strip>.png` from top to bottom: GNOME Shell
     * encodes each picture on a worker thread, and compressing a busy screen as one PNG takes
     * over a second where the strips take a fraction of it side by side. Every strip is started
     * in the same turn, so all of them show the same frame.
     */
    async _capture() {
        const dir = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'kirie']);
        GLib.mkdir_with_parents(dir, 0o700);
        GLib.chmod(dir, 0o700);
        const monitors = Main.layoutManager.monitors;
        const strips = Math.min(MAX_STRIPS,
            Math.max(1, Math.ceil(GLib.get_num_processors() / monitors.length)));
        const shots = [];
        for (const monitor of monitors) {
            // Strip edges on whole multiples of 8 logical pixels fall on whole device pixels
            // under GNOME's fractional scales.
            const height = Math.max(STRIP_STEP,
                Math.ceil(monitor.height / strips / STRIP_STEP) * STRIP_STEP);
            for (let strip = 0, top = 0; top < monitor.height; strip++, top += height) {
                const area = [monitor.x, monitor.y + top, monitor.width,
                    Math.min(height, monitor.height - top)];
                const path = GLib.build_filenamev([dir, `shell-${monitor.index}-${strip}.png`]);
                shots.push(shoot(area, path).then(() => [monitor.index, ...area, path]));
            }
        }
        const results = await Promise.allSettled(shots);
        const failed = results.find(result => result.status === 'rejected');
        if (!failed)
            return results.map(result => result.value);
        for (const result of results) {
            if (result.status === 'fulfilled')
                GLib.unlink(result.value[5]);
        }
        throw failed.reason;
    }

    /** Puts an overlay window of a Kirie this extension started on its monitor, above all. */
    _adopt(window) {
        const client = [...this._clients.values()].find(c => c.owns_window(window));
        if (!client)
            return;
        client.hide_from_window_list?.(window);
        if (Main.overview.visible)
            Main.overview.hide();
        const actor = window.get_compositor_private();
        if (actor)
            Main.wm.skipNextEffect(actor);
        const place = () => {
            const match = OVERLAY_TITLE.exec(window.get_title() ?? '');
            if (!match)
                return false;
            const index = Number(match[1]);
            if (index < global.display.get_n_monitors())
                window.move_to_monitor(index);
            window.make_fullscreen();
            window.make_above();
            window.stick();
            window.activate(global.get_current_time());
            return true;
        };
        // GNOME 51 leaves a window that is moved or made fullscreen before it is first shown
        // stuck at its default size.
        const shownId = window.connect('shown', () => {
            window.disconnect(shownId);
            if (!place()) {
                const titleId = window.connect('notify::title', () => {
                    if (place())
                        window.disconnect(titleId);
                });
            }
        });
    }
}
