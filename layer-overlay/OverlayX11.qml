import QtQuick
import QtQuick.Window
import QtWebChannel
import QtWebEngine

// The same surface as Overlay.qml, kept above a fullscreen window by
// _NET_WM_STATE_ABOVE instead of by the layer-shell protocol. Everything below
// the window flags is identical on purpose — the page cannot tell which backend
// is under it, and a difference here would be a difference only GNOME users
// ever hit.
Window {
    id: root
    visible: true
    color: "transparent"
    // The whole screen, though the page typically paints a corner of it:
    // whatever it opens needs somewhere to open into, and what takes clicks is
    // the input region, not the size.
    //
    // A window manager shrinks this to the work area whatever it is asked for —
    // fullscreen and override-redirect both fail differently — so the surface
    // is simply not assumed to start at the screen origin. Python translates
    // the tracked window into surface coordinates before the page sees it.
    x: Screen.virtualX
    y: Screen.virtualY
    width: Screen.width
    height: Screen.height

    property bool interactive: false

    // Tool rather than Window: it keeps the surface out of the taskbar and the
    // alt-tab list, which a thing with no title bar has no business being in.
    // BypassWindowManagerHint would also stay on top and is wrong — it opts out
    // of the window manager entirely, so the geometry never follows an output
    // change.
    flags: Qt.Window
         | Qt.FramelessWindowHint
         | Qt.WindowStaysOnTopHint
         | Qt.Tool
         | Qt.WindowDoesNotAcceptFocus

    WebEngineView {
        id: view
        anchors.fill: parent
        url: overlayUrl
        backgroundColor: "transparent"
        profile: WebEngineProfile {
            offTheRecord: false
            storageName: overlayScope
            persistentStoragePath: overlayStorage
        }
        userScripts.collection: [overlayWebChannelScript]
        webChannel: WebChannel { id: channel }
        Component.onCompleted: channel.registerObject("shell", overlay)

        // read-stats may still be starting, and a failed load puts Chromium's
        // error page over the whole screen with nothing to dismiss it. So the
        // error page is off — a failed load leaves the surface as it was — and
        // the view retries until the server answers.
        settings.errorPageEnabled: false

        onLoadingChanged: function (load) {
            if (load.status === WebEngineView.LoadFailedStatus)
                retry.restart()
            else if (load.status === WebEngineView.LoadSucceededStatus)
                retry.delay = 500
        }

        Timer {
            id: retry
            // Backs off, so a server that never comes up is not polled tightly
            // for the rest of the session.
            property int delay: 500
            interval: delay
            onTriggered: {
                delay = Math.min(delay * 2, 5000)
                view.reload()
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: Qt.quit()
    }
}
