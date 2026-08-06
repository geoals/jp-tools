import QtQuick
import QtQuick.Window
import QtWebChannel
import QtWebEngine
import org.kde.layershell as LayerShell

Window {
    id: root
    visible: true
    color: "transparent"
    // Full screen, though only the bottom `overlayHeight` is painted: a
    // popup needs somewhere to open into, and a surface cannot grow past
    // its own edges. What takes clicks is the input region, not the size.
    width: Screen.width
    height: Screen.height

    // Written from Python whenever the input region changes.
    property bool interactive: false

    LayerShell.Window.scope: "vn-overlay"
    LayerShell.Window.layer: LayerShell.Window.LayerOverlay
    LayerShell.Window.anchors: LayerShell.Window.AnchorTop
                             | LayerShell.Window.AnchorBottom
                             | LayerShell.Window.AnchorLeft
                             | LayerShell.Window.AnchorRight
    // Zero: the strip lies on top of the game rather than shrinking its output.
    LayerShell.Window.exclusionZone: 0
    // OnDemand leaves the keyboard with the VN until the strip is clicked, so
    // advancing text keeps working with the overlay up.
    LayerShell.Window.keyboardInteractivity: LayerShell.Window.KeyboardInteractivityOnDemand

    // The page draws its own translucent backdrop and keeps its text opaque.
    // Fading the view instead would take the words down with the background.
    WebEngineView {
        id: view
        anchors.fill: parent
        url: overlayUrl
        backgroundColor: "transparent"

        // The page is the only thing that knows where its words are, and it
        // says so the moment they move. Registered from here rather than
        // handed over from Python: the view wants a QQmlWebChannel, which
        // PySide does not expose.
        webChannel: WebChannel { id: channel }
        Component.onCompleted: channel.registerObject("shell", overlay)

        // SIGHUP. Bypasses the cache: the point of it is an edit to
        // overlay.html, which read-stats serves straight off disk.
        Connections {
            target: overlay
            function onReloadRequested() { view.reloadAndBypassCache() }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: Qt.quit()
    }
}
