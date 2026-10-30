use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const MIAO_URL: &str =
    "https://github.com/iVnc-Org/miao/releases/download/latest/miao-rust-linux-amd64";
const MIAO_BIN: &str = "miao-rust-linux-amd64";

fn main() {
    println!("cargo:rerun-if-env-changed=IVNC_BUILD_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=IVNC_BUILD_GIT_MESSAGE");
    println!("cargo:rerun-if-env-changed=IVNC_MIAO_BIN");
    println!("cargo:rerun-if-env-changed=IVNC_REFRESH_MIAO");
    println!("cargo:rerun-if-env-changed=IVNC_MIAO_URL");
    println!("cargo:rerun-if-changed=web/ivnc/dist");

    emit_build_metadata();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is not set"));
    let out_file = out_dir.join(MIAO_BIN);

    if let Ok(path) = env::var("IVNC_MIAO_BIN") {
        copy_binary(Path::new(&path), &out_file);
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let url = env::var("IVNC_MIAO_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| MIAO_URL.to_string());
    let mut cache_key = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut cache_key);
    let cache_file = manifest_dir
        .join("target")
        .join("miao-cache")
        .join(format!("{:x}", cache_key.finish()))
        .join(MIAO_BIN);
    let refresh = env::var("IVNC_REFRESH_MIAO")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    if refresh || !cache_file.exists() {
        download_binary(&cache_file, &url);
    }
    copy_binary(&cache_file, &out_file);
}

fn emit_build_metadata() {
    let commit = env::var("IVNC_BUILD_GIT_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git_output(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    let message = env::var("IVNC_BUILD_GIT_MESSAGE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git_output(&["log", "-1", "--pretty=%s"]))
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=IVNC_BUILD_GIT_COMMIT={}", commit.trim());
    println!(
        "cargo:rustc-env=IVNC_BUILD_GIT_MESSAGE={}",
        first_line(message.trim())
    );
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or(value)
}

fn copy_binary(src: &Path, dst: &Path) {
    if !src.exists() {
        panic!("miao binary not found at {}", src.display());
    }
    println!("cargo:rerun-if-changed={}", src.display());
    fs::create_dir_all(dst.parent().expect("out parent")).expect("create OUT_DIR");
    fs::copy(src, dst).unwrap_or_else(|err| {
        panic!(
            "failed to copy miao binary from {} to {}: {}",
            src.display(),
            dst.display(),
            err
        )
    });
    fs::set_permissions(dst, fs::Permissions::from_mode(0o755))
        .expect("make embedded Miao executable");
    if env::var("HOST").ok() == env::var("TARGET").ok() {
        let output = Command::new(dst)
            .arg("--ivnc-integration-version")
            .output()
            .unwrap_or_else(|error| panic!("cannot validate Miao {}: {error}", src.display()));
        assert!(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "1",
            "Miao at {} lacks iVnc integration v1. Build the paired Miao source with scripts/build-miao.sh, then set IVNC_MIAO_BIN to that binary. Old standalone releases are incompatible.", src.display());
    } else {
        panic!("Cross compilation requires native validation of the paired Miao binary; build iVnc in a container for the target architecture");
    }
}

fn download_binary(dst: &Path, url: &str) {
    fs::create_dir_all(dst.parent().expect("cache parent")).expect("create miao cache dir");
    let tmp = dst.with_extension("tmp");
    let curl_status = Command::new("curl")
        .arg("-fsSL")
        .arg(&url)
        .arg("-o")
        .arg(&tmp)
        .status();

    let downloaded = matches!(curl_status, Ok(status) if status.success());
    let downloaded = downloaded
        || matches!(
            Command::new("wget")
                .arg("-q")
                .arg("-O")
                .arg(&tmp)
                .arg(&url)
                .status(),
            Ok(status) if status.success()
        );

    if !downloaded {
        let _ = fs::remove_file(&tmp);
        panic!(
            "failed to download miao binary from {}; install curl/wget or set IVNC_MIAO_BIN",
            url
        );
    }

    fs::rename(&tmp, dst)
        .unwrap_or_else(|err| panic!("failed to store miao binary at {}: {}", dst.display(), err));
}
