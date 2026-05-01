use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RUNTIME_ARTIFACTS_FILE: &str = "runtime-artifacts.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeArtifacts {
    pid: u32,
    paths: Vec<String>,
}

fn runtime_artifacts_file() -> PathBuf {
    config::CACHE_DIR.join(RUNTIME_ARTIFACTS_FILE)
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
    path.map(resolve_spawn_cwd)
        .map(|path| path.to_string_lossy().into_owned())
}

fn resolve_spawn_cwd(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
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
