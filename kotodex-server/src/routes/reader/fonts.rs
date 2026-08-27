//! The Japanese-capable fonts installed on this machine.
//!
//! The overlay's font list used to be eight names hardcoded in the page, which
//! on any machine but this one is a row of chips that do nothing. Only the
//! server can answer it properly: the page is a browser tab.

use axum::Json;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Hiragana, katakana and a kanji, and a font has to carry all three. Several
/// interface fonts cover kana alone, and one of those set as the reader's font
/// leaves every kanji a box.
const REQUIRED: [char; 3] = ['あ', 'ア', '漢'];

/// `GET /api/reader/fonts` — Japanese-capable font families, sorted, unique.
///
/// No fonts installed: an empty list, never an error. The panel then offers the
/// font the shell was launched with and nothing else, which is what it does
/// anyway before the answer arrives.
pub async fn fonts() -> Json<Value> {
    Json(json!({ "families": families() }))
}

/// Read once and kept: testing a family means reading its font file, and the
/// set of installed fonts does not change over a reading session.
fn families() -> &'static [String] {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES.get_or_init(scan)
}

fn scan() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    // One face per family, because Regular and Bold cover the same characters
    // and the map is what makes the answer sorted and unique.
    let mut by_family: BTreeMap<String, fontdb::ID> = BTreeMap::new();
    for face in db.faces() {
        if let Some((name, _)) = face.families.first() {
            by_family.entry(name.clone()).or_insert(face.id);
        }
    }

    by_family
        .into_iter()
        .filter(|(_, id)| covers_japanese(&db, *id))
        .map(|(name, _)| name)
        .collect()
}

fn covers_japanese(db: &fontdb::Database, id: fontdb::ID) -> bool {
    db.with_face_data(id, |data, index| {
        let Ok(face) = ttf_parser::Face::parse(data, index) else {
            return false;
        };
        declares_japanese(&face)
            .unwrap_or_else(|| REQUIRED.iter().all(|c| face.glyph_index(*c).is_some()))
    })
    .unwrap_or(false)
}

/// Windows-platform language ids: Japanese, then the other CJK locales.
const JAPANESE: u16 = 0x0411;
const OTHER_CJK: [u16; 6] = [0x0412, 0x0404, 0x0804, 0x0C04, 0x1004, 0x1404];

/// Whether the font names itself in a CJK locale, and in which — `None` when it
/// names itself in none of them.
///
/// A pan-CJK font ships every locale's family out of one file over one character
/// set, so Noto Sans CJK KR holds every kana and kanji JP does and no glyph test
/// can separate them: same `cmap`, same OS/2 code pages, no `meta` table. What
/// separates them is which locale each face gives its own name in, which is what
/// fontconfig reads too. Offering the Korean one for reading Japanese would draw
/// the Korean kanji forms.
fn declares_japanese(face: &ttf_parser::Face) -> Option<bool> {
    let mut cjk = None;
    for name in face.names() {
        if name.platform_id != ttf_parser::PlatformId::Windows {
            continue;
        }
        if name.language_id == JAPANESE {
            return Some(true);
        }
        if OTHER_CJK.contains(&name.language_id) {
            cjk = Some(false);
        }
    }
    cjk
}
