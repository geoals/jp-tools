import QtQuick
import QtQuick.Window
import QtWebChannel
import QtWebEngine
import org.kde.layershell as LayerShell

Window {
    id: root
    visible: true
    color: "transparent"
    // Full screen, though a page typically paints only part of it: whatever it
    // opens — a popup, a menu — needs somewhere to open into, and a surface
    // cannot grow past its own edges. What takes clicks is the input region,
    // not the size.
    width: Screen.width
    height: Screen.height

    LayerShell.Window.scope: overlayScope
    LayerShell.Window.layer: LayerShell.Window.LayerOverlay
    LayerShell.Window.anchors: LayerShell.Window.AnchorTop
                             | LayerShell.Window.AnchorBottom
                             | LayerShell.Window.AnchorLeft
                             | LayerShell.Window.AnchorRight
    // Zero: the surface lies on top of what is below rather than shrinking its
    // output.
    LayerShell.Window.exclusionZone: 0
    // OnDemand leaves the keyboard with the window underneath until the surface
    // is clicked, so that window keeps taking input with the overlay up.
    LayerShell.Window.keyboardInteractivity: LayerShell.Window.KeyboardInteractivityOnDemand

    // The page draws its own backdrop, so it can keep its text opaque over a
    // translucent one. Fading the view instead would take the text down with
    // the background.
    WebEngineView {
        id: view
        anchors.fill: parent
        url: overlayUrl
        backgroundColor: "transparent"
        // Named, so localStorage survives a restart: the default profile is
        // off-the-record, and everything the page persists goes with it when the
        // shell exits.
        profile: WebEngineProfile {
            // Both: a storage name alone still leaves the profile
            // off-the-record, and an off-the-record profile keeps nothing.
            offTheRecord: false
            storageName: overlayScope
            persistentStoragePath: overlayStorage
        }

        // qwebchannel.js before the page runs — see webchannel_script() for why
        // the page carries no copy of it. Built in Python because
        // WebEngineScript is a QML value type here, not a creatable element.
        userScripts.collection: [overlayWebChannelScript]

        // The page is the only thing that knows where it has drawn, and it says
        // so the moment that moves. Registered from here rather than handed
        // over from Python: the view wants a QQmlWebChannel, which PySide does
        // not expose.
        webChannel: WebChannel { id: channel }

        Component.onCompleted: channel.registerObject("shell", overlay)

        // kotodex-server may still be starting, and a failed load puts Chromium's
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
