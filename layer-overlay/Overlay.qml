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

    // Written from Python whenever the input region changes.
    property bool interactive: false

    // The surface shrunk onto the tracked window, as insets from each edge of
    // the output in logical pixels. Written from Python, which is where the
    // window's rectangle is known; zero on every side is the whole output,
    // which is what a page with no window to sit on gets.
    //
    // Sized this way rather than by width/height: a surface anchored to all
    // four edges is stretched to the output and its own size ignored, and
    // dropping anchors to size it directly would leave the compositor choosing
    // where to put it.
    property int insetLeft: 0
    property int insetTop: 0
    property int insetRight: 0
    property int insetBottom: 0

    LayerShell.Window.scope: overlayScope
    LayerShell.Window.layer: LayerShell.Window.LayerOverlay
    LayerShell.Window.anchors: LayerShell.Window.AnchorTop
                             | LayerShell.Window.AnchorBottom
                             | LayerShell.Window.AnchorLeft
                             | LayerShell.Window.AnchorRight
    // Zero: the surface lies on top of what is below rather than shrinking its
    // output.
    LayerShell.Window.exclusionZone: 0
    LayerShell.Window.margins: ({
        left: root.insetLeft,
        top: root.insetTop,
        right: root.insetRight,
        bottom: root.insetBottom,
    })
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
        // off-the-record, and everything the page persists went with it every
        // time the shell exited.
        profile: WebEngineProfile {
            // Both: a storage name alone leaves the profile off-the-record,
            // which is the whole defect — it keeps nothing.
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
    }

    Shortcut {
        sequence: "Escape"
        onActivated: Qt.quit()
    }
}
