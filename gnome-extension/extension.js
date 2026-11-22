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
      <arg name="args" type="as" direction="in"/>
    </method>
    <method name="Screens">
      <arg name="screens" type="a(uiiiis)" direction="out"/>
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
            () => this._launch([]));
        this._exported = Gio.DBusExportedObject.wrapJSObject(INTERFACE, {
            Capture: args => this._launch(args),
            ScreensAsync: (params, invocation) => this._screens(invocation),
            SetClipboardAsync: (params, invocation) => this._setClipboard(params, invocation),
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

    _launch(args) {
        const configured = this._settings.get_strv('command');
        const command = configured.length > 0 ? configured : ['kirie'];
        if (!GLib.find_program_in_path(command[0])) {
            Main.notifyError('Kirie is not installed',
                `“${command[0]}” was not found. Install Kirie to take screenshots with it.`);
            return;
        }
        const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
        launcher.setenv('KIRIE_SHELL_LAUNCHED', '1', true);
        const argv = [...command, 'capture', ...args.filter(arg => typeof arg === 'string')];
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

    _screens(invocation) {
        if (!this._trusted(invocation))
            return;
        this._capture().then(screens => {
            invocation.return_value(new GLib.Variant('(a(uiiiis))', [screens]));
        }).catch(error => {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', error.message);
        });
    }

    /** One PNG per monitor in the user's runtime folder, with where the monitor sits. */
    async _capture() {
        const dir = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'kirie']);
        GLib.mkdir_with_parents(dir, 0o700);
        const screens = [];
        for (const monitor of Main.layoutManager.monitors) {
            const path = GLib.build_filenamev([dir, `shell-${monitor.index}.png`]);
            const file = Gio.File.new_for_path(path);
            const stream = file.replace(null, false, Gio.FileCreateFlags.PRIVATE, null);
            const shooter = new Shell.Screenshot();
            await shooter.screenshot_area(
                monitor.x, monitor.y, monitor.width, monitor.height, stream);
            stream.close(null);
            screens.push([monitor.index, monitor.x, monitor.y, monitor.width, monitor.height,
                path]);
        }
        return screens;
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
        if (!place()) {
            const id = window.connect('notify::title', () => {
                if (place())
                    window.disconnect(id);
            });
        }
    }
}
