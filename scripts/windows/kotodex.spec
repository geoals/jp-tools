# -*- mode: python ; coding: utf-8 -*-
# The three Python components, packaged into .exes that share one copy of Qt.
#
# Run by build-installer.ps1; not meant to be run by hand.
#
# One spec rather than three PyInstaller runs because all three are PySide6
# applications and Qt is most of what they weigh. Three runs give three trees,
# and an .exe finds its `_internal` beside itself, so the launcher's copy of
# Qt6Core/Gui/Widgets cannot simply be deleted afterwards - three `Analysis`
# objects feeding one `COLLECT` is what makes the three .exes share a directory.
#
# `base_library.zip` is the one file the three do not each get their own of.
# COLLECT takes whichever it sees first, which is safe only because its contents
# are Python's own bootstrap module set, fixed by the interpreter version rather
# than by the application. If a future PyInstaller makes it per-application, all
# three .exes would need testing again.

import os

REPO = os.path.dirname(os.path.dirname(SPECPATH))
ICON = os.path.join(REPO, 'kotodex', 'icons', 'kotodex.ico')


def at(*parts):
    return os.path.join(REPO, *parts)


overlay = Analysis(
    [at('kotodex-server', 'overlay', 'vn-overlay.py')],
    pathex=[at('layer-overlay')],
    datas=[
        (at('layer-overlay', 'Overlay.qml'), '.'),
        (at('layer-overlay', 'OverlayWindow.qml'), '.'),
    ],
    # Imported behind `if BACKEND == backend.WINDOWS`, which PyInstaller's static
    # analysis does see - named anyway, because losing either is a crash on the
    # first line rather than a build error.
    hiddenimports=['winfocus', 'wininput', 'winwatch'],
)

source = Analysis([at('sources', 'textractor', 'vn-ws-logger.py')])

launcher = Analysis(
    [at('kotodex', 'kotodex.py')],
    pathex=[at('kotodex')],
    hiddenimports=['single_instance', 'tray'],
)

# --------------------------------------------------------------------- prune --

# Qt modules nothing here imports. Dropped from the table before COLLECT rather
# than deleted after it, so they are never copied at all.
#
# Safe because none of them is a load-time dependency of the sixteen Qt DLLs the
# three applications do pull in: Qt loads each of these through a QML or plugin
# import, so what is gone is only unreachable, never a missing symbol at startup.
# Check that again before adding a name - `objdump -p` over the wheel is how the
# list was drawn.
#
# The QtQuick.Controls family stays, unused as it looks: QtWebEngine's own QML
# delegates import it to draw a context menu and a file picker.
DROP_DLL = (
    'Qt63D', 'Qt6Bluetooth', 'Qt6CanvasPainter', 'Qt6Charts', 'Qt6DataVisualization',
    'Qt6Designer', 'Qt6Graphs', 'Qt6Help', 'Qt6HttpServer', 'Qt6Location', 'Qt6Lottie',
    'Qt6Multimedia', 'Qt6NetworkAuth', 'Qt6Nfc', 'Qt6Pdf', 'Qt6Quick3D',
    'Qt6RemoteObjects', 'Qt6Scxml', 'Qt6Sensors', 'Qt6SerialBus', 'Qt6SerialPort',
    'Qt6SpatialAudio', 'Qt6Sql', 'Qt6StateMachine', 'Qt6TextToSpeech', 'Qt6UiTools',
    'Qt6VirtualKeyboard', 'Qt6WebView', 'Qt6QuickTest', 'Qt6Test',
)
DROP_QML = (
    'Qt3D', 'QtCharts', 'QtCore5Compat', 'QtDataVisualization', 'QtGraphs',
    'QtLocation', 'QtMultimedia', 'QtPositioning', 'QtQuick3D', 'QtRemoteObjects',
    'QtScxml', 'QtSensors', 'QtSpatialAudio', 'QtTest', 'QtTextToSpeech',
    'QtVirtualKeyboard', 'QtWebView', 'QtQuick/Scene2D', 'QtQuick/Scene3D',
    'QtQuick/VirtualKeyboard', 'QtQml/StateMachine',
)
DROP_PLUGIN = (
    'sceneparsers', 'geometryloaders', 'renderplugins', 'assetimporters',
    'multimedia', 'texttospeech', 'position', 'sensors', 'sqldrivers',
    'virtualkeyboard', 'designer', 'webview', 'qmltooling', 'printsupport',
)

# Qt ships Chromium's resource archives twice, once built for debug. Only remote
# debugging reads the debug set, and the overlay exposes no port for it.
DROP_SUFFIX = ('.debug.pak', '.debug.bin')

# Qt's own dialog strings, in every language Qt ships. The page is Japanese and
# English and supplies its own text; what is left here is the language of a file
# picker nothing opens.
KEEP_QM = ('en', 'ja')
# Chromium names its own by full locale, so this needs its own list. en-US rather
# than en-GB or any other: it is Chromium's fallback, and without it an English
# machine has no locale it can load.
KEEP_LOCALE = ('en-US', 'ja')


def wanted(entry):
    dest = entry[0].replace('\\', '/')
    name = dest.rsplit('/', 1)[-1]
    if name.endswith(DROP_SUFFIX):
        return False
    # Matched with the `lib` prefix off, so the one list covers `Qt6Charts.dll` and
    # `libQt6Charts.so.6` alike. Only Windows ships this, but a spec that filters
    # nothing on Linux cannot be checked anywhere but in CI.
    if name.removeprefix('lib').startswith(DROP_DLL):
        return False
    # Only remote debugging reads these, and the overlay opens no port for it.
    if name.startswith('qtwebengine_devtools_resources'):
        return False
    parts = dest.split('/')
    if 'qml' in parts:
        tail = '/'.join(parts[parts.index('qml') + 1:])
        if tail.startswith(DROP_QML):
            return False
    if 'plugins' in parts and any(p in dest for p in DROP_PLUGIN):
        return False
    if 'translations' in parts:
        if 'qtwebengine_locales' in parts:
            return name.removesuffix('.pak') in KEEP_LOCALE
        if name.endswith('.qm'):
            return name.removesuffix('.qm').rsplit('_', 1)[-1] in KEEP_QM
    return True


def keep(*tables):
    merged = []
    for table in tables:
        merged.extend(e for e in table if wanted(e))
    return merged


# ------------------------------------------------------------------ package --

# --windowed for the overlay and the launcher: one draws its own window, the other
# is a tray and nothing else.
#
# The source gets a console. A process without one receives no CTRL_C_EVENT or
# CTRL_BREAK_EVENT at all, and that event is its only warning that it is being
# shut down - it needs one to send Textractor's WebSocket plugin a proper close
# frame, which is what keeps an abortive disconnect from crashing Textractor
# itself. The launcher hides the window.
exes = [
    EXE(PYZ(overlay.pure, overlay.zipped_data), overlay.scripts, [],
        exclude_binaries=True, name='kotodex-overlay', console=False, icon=ICON),
    EXE(PYZ(source.pure, source.zipped_data), source.scripts, [],
        exclude_binaries=True, name='kotodex-source', console=True, icon=ICON),
    EXE(PYZ(launcher.pure, launcher.zipped_data), launcher.scripts, [],
        exclude_binaries=True, name='kotodex', console=False, icon=ICON),
]

COLLECT(
    *exes,
    keep(overlay.binaries, source.binaries, launcher.binaries),
    keep(overlay.datas, source.datas, launcher.datas),
    name='app',
)
