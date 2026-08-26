//! Where the files that are not binaries live.

use std::path::{Path, PathBuf};

/// The directory the tools were installed into: the workspace root in a
/// checkout, the unpacked tarball otherwise.
///
/// Assets sit at fixed paths under it — the overlay page, the dashboard's
/// static files, `backend.py`, `vn-capture.sh`.
///
/// Three answers, in order. `KOTODEX_ROOT` wins, which is how the launcher
/// tells its children where it was unpacked. Otherwise the binary's own
/// location: it is installed at `<root>/target/release/<bin>`, and that holds
/// for a tarball and a checkout alike. The compiled-in workspace is last,
/// because it names the machine the binary was *built* on — for a release that
/// is a CI container that does not exist here.
pub fn install_root() -> PathBuf {
    if let Ok(root) = std::env::var("KOTODEX_ROOT") {
        return PathBuf::from(root);
    }
    if let Some(root) = root_from_exe() {
        return root;
    }
    build_workspace()
}

/// `<root>/target/release/jp-dict` → `<root>`, when that directory holds the
/// assets. The layout check is what keeps a binary copied to `~/.local/bin`
/// from claiming the home directory as the root.
fn root_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let root = exe.parent()?.parent()?.parent()?;
    root.join("kotodex-server/static").is_dir().then(|| root.to_path_buf())
}

fn build_workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("jp-core always has a workspace parent")
        .to_path_buf()
}
