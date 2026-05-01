use anyhow::Context;
use mux::pane::CachePolicy;
use mux::Mux;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RENAMED_TABS_SESSION_FILE: &str = "renamed-tabs-session.json";
const RUNTIME_ARTIFACTS_FILE: &str = "runtime-artifacts.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedWindow {
    pub tabs: Vec<SavedTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedSession {
    pub windows: Vec<SavedWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeArtifacts {
    pid: u32,
    paths: Vec<String>,
}

fn session_file() -> PathBuf {
    config::DATA_DIR.join(RENAMED_TABS_SESSION_FILE)
}

fn runtime_artifacts_file() -> PathBuf {
    config::CACHE_DIR.join(RUNTIME_ARTIFACTS_FILE)
}

pub fn save_current_session() -> anyhow::Result<()> {
    save_current_session_impl(None)
}

pub fn save_current_session_excluding_window(
    window_id: mux::window::WindowId,
) -> anyhow::Result<()> {
    save_current_session_impl(Some(window_id))
}

fn save_current_session_impl(
    exclude_window_id: Option<mux::window::WindowId>,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&*config::DATA_DIR)?;

    let mux = Mux::get();
    let mut session = SavedSession::default();

    for window_id in mux.iter_windows() {
        if Some(window_id) == exclude_window_id {
            continue;
        }
        let Some(window) = mux.get_window(window_id) else {
            continue;
        };

        let mut saved_tabs = vec![];
        for tab in window.iter() {
            let title = tab.get_title();
            if title.is_empty() {
                continue;
            }

            let cwd = tab.get_spawn_cwd().or_else(|| {
                tab.get_active_pane()
                    .and_then(|pane| pane.get_current_working_dir(CachePolicy::AllowStale))
                    .and_then(url_to_path_string)
            });

            saved_tabs.push(SavedTab { title, cwd });
        }

        if !saved_tabs.is_empty() {
            session.windows.push(SavedWindow { tabs: saved_tabs });
        }
    }

    let path = session_file();
    if session.windows.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }

    std::fs::write(path, serde_json::to_string_pretty(&session)?)?;
    Ok(())
}

pub fn load_saved_session() -> anyhow::Result<Option<SavedSession>> {
    let path = session_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let session = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(session))
}

pub fn cleanup_prior_runtime_artifacts() -> anyhow::Result<()> {
    let path = runtime_artifacts_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let state: RuntimeArtifacts = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;

    if process_is_alive(state.pid) {
        return Ok(());
    }

    for entry in state.paths {
        let artifact = PathBuf::from(entry);
        if !artifact.exists() {
            continue;
        }
        let res = if artifact.is_dir() {
            std::fs::remove_dir_all(&artifact)
        } else {
            std::fs::remove_file(&artifact)
        };
        if let Err(err) = res {
            log::warn!("failed to remove stale artifact {}: {err:#}", artifact.display());
        }
    }

    let _ = std::fs::remove_file(path);
    Ok(())
}

pub fn save_runtime_artifacts(paths: &[PathBuf]) -> anyhow::Result<()> {
    std::fs::create_dir_all(&*config::CACHE_DIR)?;
    let state = RuntimeArtifacts {
        pid: std::process::id(),
        paths: paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    };
    std::fs::write(runtime_artifacts_file(), serde_json::to_string_pretty(&state)?)?;
    Ok(())
}

pub fn effective_spawn_cwd(path: Option<&Path>) -> Option<String> {
    path.map(|path| path.to_string_lossy().into_owned())
}

fn url_to_path_string(url: url::Url) -> Option<String> {
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use winapi::shared::minwindef::FALSE;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::minwinbase::STILL_ACTIVE;
    use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess};
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if handle.is_null() {
        return true;
    }

    let mut status = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut status) != 0 };
    unsafe {
        CloseHandle(handle);
    }

    if !ok {
        return true;
    }

    status == STILL_ACTIVE
}

#[cfg(not(windows))]
fn process_is_alive(_pid: u32) -> bool {
    true
}
