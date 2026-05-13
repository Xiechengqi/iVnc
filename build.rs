use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MIAO_URL: &str =
    "https://github.com/YUxiangLuo/miao/releases/download/v0.18.4/miao-rust-linux-amd64";
const MIAO_BIN: &str = "miao-rust-linux-amd64";

fn main() {
    println!("cargo:rerun-if-env-changed=IVNC_MIAO_BIN");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is not set"));
    let out_file = out_dir.join(MIAO_BIN);

    if let Ok(path) = env::var("IVNC_MIAO_BIN") {
        copy_binary(Path::new(&path), &out_file);
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let cache_file = manifest_dir
        .join("target")
        .join("miao-cache")
        .join(MIAO_BIN);
    if !cache_file.exists() {
        download_binary(&cache_file);
    }
    copy_binary(&cache_file, &out_file);
}

fn copy_binary(src: &Path, dst: &Path) {
    if !src.exists() {
        panic!("miao binary not found at {}", src.display());
    }
    fs::create_dir_all(dst.parent().expect("out parent")).expect("create OUT_DIR");
    fs::copy(src, dst).unwrap_or_else(|err| {
        panic!(
            "failed to copy miao binary from {} to {}: {}",
            src.display(),
            dst.display(),
            err
        )
    });
}

fn download_binary(dst: &Path) {
    fs::create_dir_all(dst.parent().expect("cache parent")).expect("create miao cache dir");
    let tmp = dst.with_extension("tmp");
    let curl_status = Command::new("curl")
        .arg("-fsSL")
        .arg(MIAO_URL)
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
                .arg(MIAO_URL)
                .status(),
            Ok(status) if status.success()
        );

    if !downloaded {
        let _ = fs::remove_file(&tmp);
        panic!(
            "failed to download miao binary from {}; install curl/wget or set IVNC_MIAO_BIN",
            MIAO_URL
        );
    }

    fs::rename(&tmp, dst)
        .unwrap_or_else(|err| panic!("failed to store miao binary at {}: {}", dst.display(), err));
}
