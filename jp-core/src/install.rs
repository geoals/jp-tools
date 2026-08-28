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

/// The directory the data lives in: the two databases, the covers, the Sudachi
/// dictionary and the VAD model that `setup.sh` downloads.
///
/// The per-file env vars (`KOTODEX_KNOWLEDGE_DB_PATH` and the rest) still win
/// over this wherever they are read — this is only the default under them.
pub fn data_dir() -> PathBuf {
    user_data_root().join("kotodex")
}

#[cfg(windows)]
fn user_data_root() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA not set"))
}

#[cfg(not(windows))]
fn user_data_root() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".local/share")
}

/// `<root>/target/release/jp-dict` → `<root>`, when that directory holds the
/// assets. The layout check is what keeps a binary copied to `~/.local/bin`
/// from claiming the home directory as the root.
fn root_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let root = exe.parent()?.parent()?.parent()?;
    root.join("kotodex-server/static")
        .is_dir()
        .then(|| root.to_path_buf())
}

fn build_workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("jp-core always has a workspace parent")
        .to_path_buf()
}
