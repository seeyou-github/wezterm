fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        use anyhow::Context as _;
        use std::io::Write;
        use std::path::Path;
        let profile = std::env::var("PROFILE").unwrap();
        let repo_dir = std::env::current_dir()
            .ok()
            .and_then(|cwd| cwd.parent().map(|p| p.to_path_buf()))
            .unwrap();
        let exe_output_dir = repo_dir.join("target").join(profile);
        let windows_dir = repo_dir.join("assets").join("windows");

        let conhost_dir = windows_dir.join("conhost");
        for name in &["conpty.dll", "OpenConsole.exe"] {
            let dest_name = exe_output_dir.join(name);
            let src_name = conhost_dir.join(name);

            if !dest_name.exists() {
                std::fs::copy(&src_name, &dest_name)
                    .context(format!(
                        "copy {} -> {}",
                        src_name.display(),
                        dest_name.display()
                    ))
                    .unwrap();
            }
        }

        let angle_dir = windows_dir.join("angle");
        for name in &["libEGL.dll", "libGLESv2.dll"] {
            let dest_name = exe_output_dir.join(name);
            let src_name = angle_dir.join(name);

            if !dest_name.exists() {
                std::fs::copy(&src_name, &dest_name)
                    .context(format!(
                        "copy {} -> {}",
                        src_name.display(),
                        dest_name.display()
                    ))
                    .unwrap();
            }
        }

        {
            let dest_name = exe_output_dir.join("wezterm-windows.conf");
            let src_name = windows_dir.join("wezterm-windows.conf");
            if !dest_name.exists() {
                std::fs::copy(&src_name, &dest_name)
                    .context(format!(
                        "copy {} -> {}",
                        src_name.display(),
                        dest_name.display()
                    ))
                    .unwrap();
            }
        }

        {
            let dest_mesa = exe_output_dir.join("mesa");
            let _ = std::fs::create_dir(&dest_mesa);
            let dest_name = dest_mesa.join("opengl32.dll");
            let src_name = windows_dir.join("mesa").join("opengl32.dll");
            if !dest_name.exists() {
                std::fs::copy(&src_name, &dest_name)
                    .context(format!(
                        "copy {} -> {}",
                        src_name.display(),
                        dest_name.display()
                    ))
                    .unwrap();
            }
        }

        // If a file named `.tag` is present, we'll take its contents for the
        // version number that we report in wezterm -h.
        let mut ci_tag = String::new();
        if let Ok(tag) = std::fs::read("../.tag") {
            if let Ok(s) = String::from_utf8(tag) {
                ci_tag = s.trim().to_string();
                println!("cargo:rerun-if-changed=../.tag");
            }
        }
        let version = if ci_tag.is_empty() {
            let mut cmd = std::process::Command::new("git");
            cmd.args(&[
                "-c",
                "core.abbrev=8",
                "show",
                "-s",
                "--format=%cd-%h",
                "--date=format:%Y%m%d-%H%M%S",
            ]);
            if let Ok(output) = cmd.output() {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).trim().to_owned()
                } else {
                    "UNKNOWN".to_owned()
                }
            } else {
                "UNKNOWN".to_owned()
            }
        } else {
            ci_tag
        };

        let rcfile_name = Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("resource.rc");
        let mut rcfile = std::fs::File::create(&rcfile_name).unwrap();
        println!("cargo:rerun-if-changed=../assets/windows/terminal.ico");
        write!(
            rcfile,
            r#"
#include <winres.h>
// This ID is coupled with code in window/src/os/windows/window.rs
#define IDI_ICON 0x101
1 RT_MANIFEST "{win}\\manifest.manifest"
IDI_ICON ICON "{win}\\terminal.ico"
VS_VERSION_INFO VERSIONINFO
FILEVERSION     1,0,0,0
PRODUCTVERSION  1,0,0,0
FILEFLAGSMASK   VS_FFI_FILEFLAGSMASK
FILEFLAGS       0
FILEOS          VOS__WINDOWS32
FILETYPE        VFT_APP
FILESUBTYPE     VFT2_UNKNOWN
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904E4"
        BEGIN
            VALUE "CompanyName",      "Wez Furlong\0"
            VALUE "FileDescription",  "WezTerm - Wez's Terminal Emulator\0"
            VALUE "FileVersion",      "{version}\0"
            VALUE "LegalCopyright",   "Wez Furlong, MIT licensed\0"
            VALUE "InternalName",     "\0"
            VALUE "OriginalFilename", "\0"
            VALUE "ProductName",      "WezTerm\0"
            VALUE "ProductVersion",   "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1252
    END
END
"#,
            win = windows_dir.display().to_string().replace("\\", "\\\\"),
            version = version,
        )
        .unwrap();
        drop(rcfile);

        compile_windows_resource(&repo_dir, &rcfile_name);
    }

    #[cfg(target_os = "macos")]
    {
        use anyhow::Context as _;
        let profile = std::env::var("PROFILE").unwrap();
        let repo_dir = std::env::current_dir()
            .ok()
            .and_then(|cwd| cwd.parent().map(|p| p.to_path_buf()))
            .unwrap();

        // We need to copy the plist to avoid the UNUserNotificationCenter asserting
        // due to not finding the application bundle
        let src_plist = repo_dir
            .join("assets")
            .join("macos")
            .join("WezTerm.app")
            .join("Contents")
            .join("Info.plist");
        let build_target_dir = std::env::var("CARGO_TARGET_DIR")
            .and_then(|s| Ok(std::path::PathBuf::from(s)))
            .unwrap_or(repo_dir.join("target").join(profile));
        let dest_plist = build_target_dir.join("Info.plist");
        println!("cargo:rerun-if-changed=assets/macos/WezTerm.app/Contents/Info.plist");

        std::fs::copy(&src_plist, &dest_plist)
            .context(format!(
                "copy {} -> {}",
                src_plist.display(),
                dest_plist.display()
            ))
            .unwrap();
    }
}

#[cfg(windows)]
fn apply_windows_build_env(repo_dir: &std::path::Path) {
    let path = repo_dir.join("wezterm-build-env.conf");
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            std::env::set_var(key.trim(), value.trim());
        }
    }
}

#[cfg(windows)]
fn compile_windows_resource(repo_dir: &std::path::Path, rcfile_name: &std::path::Path) {
    apply_windows_build_env(repo_dir);

    let resfile_name = std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap())
        .join("resource.res");
    let rc = find_rc_exe().unwrap_or_else(|| "rc.exe".into());
    let mut cmd = std::process::Command::new(&rc);
    cmd.arg("/nologo")
        .arg(format!("/fo{}", resfile_name.display()));
    for include_dir in find_rc_include_dirs() {
        cmd.arg(format!("/I{}", include_dir.display()));
    }
    let status = cmd
        .arg(rcfile_name)
        .status()
        .unwrap_or_else(|err| panic!("failed to run {:?}: {:#}", rc, err));

    assert!(
        status.success(),
        "{:?} failed compiling {}",
        rc,
        rcfile_name.display()
    );
    println!("cargo:rustc-link-arg-bins={}", resfile_name.display());
}

#[cfg(windows)]
fn find_rc_exe() -> Option<std::ffi::OsString> {
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-env-changed=PATH");
    println!("cargo:rerun-if-env-changed=WindowsSdkVerBinPath");
    println!("cargo:rerun-if-env-changed=WindowsSdkBinPath");
    println!("cargo:rerun-if-env-changed=WindowsSdkDir");
    println!("cargo:rerun-if-env-changed=WindowsSDKVersion");
    println!("cargo:rerun-if-env-changed=ProgramFiles(x86)");
    println!("cargo:rerun-if-env-changed=ProgramFiles");

    if let Some(rc) = std::env::var_os("RC").filter(|value| !value.is_empty()) {
        return Some(rc);
    }

    let arch = if std::env::var("TARGET")
        .map(|target| target.contains("aarch64"))
        .unwrap_or(false)
    {
        "arm64"
    } else if std::env::var("TARGET")
        .map(|target| target.contains("x86_64"))
        .unwrap_or(false)
    {
        "x64"
    } else {
        "x86"
    };

    let mut candidates = vec![];

    if let Some(path) = std::env::var_os("WindowsSdkVerBinPath") {
        let path = std::path::PathBuf::from(path);
        candidates.push(path.join("rc.exe"));
        candidates.push(path.join(arch).join("rc.exe"));
    }

    if let Some(path) = std::env::var_os("WindowsSdkBinPath") {
        let path = std::path::PathBuf::from(path);
        candidates.push(path.join("rc.exe"));
        candidates.push(path.join(arch).join("rc.exe"));
    }

    if let Some(dir) = std::env::var_os("WindowsSdkDir") {
        let dir = std::path::PathBuf::from(dir);
        if let Some(version) = std::env::var_os("WindowsSDKVersion") {
            let version = std::path::PathBuf::from(version);
            candidates.push(dir.join("bin").join(&version).join(arch).join("rc.exe"));
            candidates.push(dir.join("bin").join(version).join("rc.exe"));
        }
    }

    for base_var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = std::env::var_os(base_var) {
            let kits_root = std::path::PathBuf::from(base).join("Windows Kits");
            for kit in ["10", "8.1"] {
                let bin_dir = kits_root.join(kit).join("bin");
                if !bin_dir.is_dir() {
                    continue;
                }

                candidates.push(bin_dir.join(arch).join("rc.exe"));
                candidates.push(bin_dir.join("rc.exe"));

                if let Ok(entries) = std::fs::read_dir(&bin_dir) {
                    let mut version_dirs: Vec<_> = entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.path())
                        .filter(|path| path.is_dir())
                        .collect();
                    version_dirs.sort();
                    version_dirs.reverse();

                    for version_dir in version_dirs {
                        candidates.push(version_dir.join(arch).join("rc.exe"));
                        candidates.push(version_dir.join("rc.exe"));
                    }
                }
            }
        }
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.into_os_string())
}

#[cfg(windows)]
fn find_rc_include_dirs() -> Vec<std::path::PathBuf> {
    println!("cargo:rerun-if-env-changed=INCLUDE");

    let mut dirs = vec![];

    if let Some(include) = std::env::var_os("INCLUDE") {
        dirs.extend(std::env::split_paths(&include).filter(|path| path.is_dir()));
    }

    if !dirs.is_empty() {
        return dirs;
    }

    println!("cargo:rerun-if-env-changed=WindowsSdkDir");
    println!("cargo:rerun-if-env-changed=WindowsSDKVersion");
    println!("cargo:rerun-if-env-changed=ProgramFiles(x86)");
    println!("cargo:rerun-if-env-changed=ProgramFiles");

    if let Some(dir) = std::env::var_os("WindowsSdkDir") {
        let include_root = std::path::PathBuf::from(dir).join("Include");
        if let Some(version) = std::env::var_os("WindowsSDKVersion") {
            let version = std::path::PathBuf::from(version);
            dirs.extend(windows_sdk_include_dirs_for(&include_root.join(version)));
        }
    }

    for base_var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = std::env::var_os(base_var) {
            let include_root = std::path::PathBuf::from(base)
                .join("Windows Kits")
                .join("10")
                .join("Include");
            if !include_root.is_dir() {
                continue;
            }

            if let Ok(entries) = std::fs::read_dir(&include_root) {
                let mut version_dirs: Vec<_> = entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect();
                version_dirs.sort();
                version_dirs.reverse();

                for version_dir in version_dirs {
                    let candidate_dirs = windows_sdk_include_dirs_for(&version_dir);
                    if !candidate_dirs.is_empty() {
                        dirs.extend(candidate_dirs);
                        return dirs;
                    }
                }
            }
        }
    }

    dirs
}

#[cfg(windows)]
fn windows_sdk_include_dirs_for(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![];
    for name in ["um", "shared"] {
        let dir = root.join(name);
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
    dirs
}
