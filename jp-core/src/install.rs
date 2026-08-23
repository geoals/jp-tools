//! Where the files that are not binaries live.

use std::path::{Path, PathBuf};

/// The directory the tools were installed into: the workspace root in a
/// checkout, the unpacked tarball otherwise.
///
/// Assets sit at fixed paths under it — the overlay page, the dashboard's
/// static files, `backend.py`, `vn-capture.sh`. The binaries are relocatable
/// and the path they were compiled in is not, so a release sets `KOTODEX_ROOT`.
/// Without it the build's own workspace is the answer, which is what a checkout
/// wants and what every test and dev run relies on.
pub fn install_root() -> PathBuf {
    if let Ok(root) = std::env::var("KOTODEX_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("jp-core always has a workspace parent")
        .to_path_buf()
}
