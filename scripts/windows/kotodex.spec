# -*- mode: python ; coding: utf-8 -*-

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
    hiddenimports=['winfocus', 'wininput', 'winwatch'],
)

source = Analysis([at('sources', 'textractor', 'vn-ws-logger.py')])

launcher = Analysis(
    [at('kotodex', 'kotodex.py')],
    pathex=[at('kotodex')],
    hiddenimports=['single_instance', 'tray'],
)

# --------------------------------------------------------------------- prune --

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
DROP_SUFFIX = ('.debug.pak', '.debug.bin')

KEEP_QM = ('en', 'ja')
KEEP_LOCALE = ('en-US', 'ja')


def wanted(entry):
    dest = entry[0].replace('\\', '/')
    name = dest.rsplit('/', 1)[-1]
    if name.endswith(DROP_SUFFIX):
        return False
    if name.removeprefix('lib').startswith(DROP_DLL):
        return False
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
