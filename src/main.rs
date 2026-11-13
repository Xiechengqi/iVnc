//! iVnc - Main entry point
//!
//! Wayland compositor + WebRTC streaming using smithay and GStreamer.

mod apps;
mod args;
mod audio;
mod capabilities;
mod clipboard;
mod compositor;
mod config;
mod console_config;
mod display;
mod file_upload;
mod gstreamer;
mod input;
mod paths;
mod proxy_panel;
mod remote;
mod runtime_settings;
mod terminal;
mod transport;
mod web;
mod webrtc;

use ::gstreamer as gst;
use args::{Cli, Command, RunArgs};
use audio::{run_audio_capture, AudioConfig as RuntimeAudioConfig};
use base64::Engine;
use clap::Parser;
use compositor::{Compositor, HeadlessBackend};
use config::Config;
use gstreamer::PipelineConfig;
use input::{InputEvent, InputEventData};
use log::{error, info, warn};
use smithay::reexports::wayland_server::Resource;
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use web::shared::RenderState;
use webrtc::SessionManager;

const VIEWER_QUIET_IDLE_AFTER: Duration = Duration::from_secs(30);

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Look up a .desktop file whose StartupWMClass matches the given app_id,
/// and return its Name= value. Returns None if no match found.
fn resolve_display_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }

    let apps_dir = crate::paths::applications_dir();
    let entries = std::fs::read_dir(&apps_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let mut name = None;
        let mut wm_class = None;
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("Name=") {
                name = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
                wm_class = Some(v.to_string());
            }
        }
        if wm_class.as_deref() == Some(app_id) {
            return name;
        }
    }
    None
}

/// Check that required shared libraries are present on the system.
/// Prints friendly install instructions and exits if any are missing.
fn check_runtime_deps() {
    let deps: &[(&str, &str)] = &[
        ("libgstreamer-1.0.so.0", "libgstreamer1.0-0"),
        ("libgstapp-1.0.so.0", "libgstreamer-plugins-base1.0-0"),
        ("libpixman-1.so.0", "libpixman-1-0"),
        ("libxkbcommon.so.0", "libxkbcommon0"),
        #[cfg(feature = "pulseaudio")]
        ("libpulse-simple.so.0", "libpulse0"),
        #[cfg(any(feature = "pulseaudio", feature = "audio"))]
        ("libopus.so.0", "libopus0"),
    ];

    let mut missing = Vec::new();
    for &(soname, pkg) in deps {
        let cstr = std::ffi::CString::new(soname).unwrap();
        let handle = unsafe { libc::dlopen(cstr.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            missing.push((soname, pkg));
        } else {
            unsafe {
                libc::dlclose(handle);
            }
        }
    }

    if !missing.is_empty() {
        eprintln!("ERROR: Missing runtime libraries:");
        for (soname, pkg) in &missing {
            eprintln!("  {} (package: {})", soname, pkg);
        }
        let pkgs: Vec<&str> = missing.iter().map(|(_, p)| *p).collect();
        eprintln!("\nInstall with:\n  apt-get install {}", pkgs.join(" "));
        std::process::exit(1);
    }

    // Check GStreamer plugins
    if gst::init().is_err() {
        eprintln!("ERROR: Failed to initialize GStreamer");
        std::process::exit(1);
    }

    let gst_plugins: &[(&str, &str)] = &[
        ("videoconvert", "gstreamer1.0-plugins-base"),
        ("appsrc", "gstreamer1.0-plugins-base"),
        ("rtph264pay", "gstreamer1.0-plugins-good"),
        ("rtpvp8pay", "gstreamer1.0-plugins-good"),
        ("x264enc", "gstreamer1.0-plugins-ugly"),
        ("ximagesrc", "gstreamer1.0-x"),
    ];

    let mut missing_plugins = Vec::new();
    for &(element, pkg) in gst_plugins {
        if gst::ElementFactory::find(element).is_none() {
            missing_plugins.push((element, pkg));
        }
    }

    if !missing_plugins.is_empty() {
        eprintln!("WARNING: Missing GStreamer plugins:");
        for (element, pkg) in &missing_plugins {
            eprintln!("  {} (package: {})", element, pkg);
        }
        let mut pkgs: Vec<&str> = missing_plugins.iter().map(|(_, p)| *p).collect();
        pkgs.sort_unstable();
        pkgs.dedup();
        eprintln!("\nInstall with:\n  apt-get install {}", pkgs.join(" "));
    }
}

/// Ensure PulseAudio daemon is running so audio capture can connect.
/// If not running, starts it with `--exit-idle-time=-1` to keep it alive.
#[cfg(feature = "pulseaudio")]
fn ensure_pulseaudio() {
    use std::process::Command;

    // Check if already running
    let status = Command::new("pulseaudio").arg("--check").status();
    if let Ok(s) = status {
        if s.success() {
            info!("PulseAudio already running");
            return;
        }
    }

    info!("PulseAudio not running, starting daemon...");
    match Command::new("pulseaudio")
        .args(["--start", "--exit-idle-time=-1"])
        .status()
    {
        Ok(s) if s.success() => info!("PulseAudio daemon started"),
        Ok(s) => warn!("PulseAudio start exited with: {}", s),
        Err(e) => warn!("Failed to start PulseAudio: {}", e),
    }
}

fn main() {
    // Owner of a desktop application's process tree; see `worker::reap`.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--app-reaper")) {
        std::process::exit(remote::worker::reap(std::env::args_os().skip(2).collect()));
    }
    let cli = Cli::parse();

    // Handle `ivnc env` subcommand early (no compositor needed)
    if let Some(Command::Env) = &cli.command {
        handle_env_command();
        return;
    }

    // Extract RunArgs: from `ivnc run ...` or bare `ivnc ...`
    let args = match cli.command {
        Some(Command::Run(a)) => a,
        _ => cli.run_args,
    };

    if let Some(path) = &args.app_supervisor {
        let runtime = tokio::runtime::Runtime::new().expect("supervisor runtime");
        if let Err(error) = runtime.block_on(remote::worker::supervise(path)) {
            eprintln!("Application runtime failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    let worker_config = args.app_worker.as_ref().map(|path| {
        remote::worker::prepare(path).unwrap_or_else(|e| {
            eprintln!("Application runtime failed: {e}");
            std::process::exit(1)
        })
    });
    check_runtime_deps();
    let log_level = if args.verbose { "debug" } else { "info" };
    env_logger::Builder::new()
        .parse_filters(&format!(
            "ivnc={},smithay={},str0m=warn,webrtc=warn,webrtc_ice=warn",
            log_level, log_level
        ))
        .init();

    info!("ivnc v{} starting", env!("CARGO_PKG_VERSION"));

    let mut config = match worker_config
        .clone()
        .map(Ok)
        .unwrap_or_else(|| args.load_config())
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    if worker_config.is_none() {
        apply_cli_overrides(&mut config, &args);
    }
    remote::worker::set_parent_config(config.clone());

    if let Err(e) = config.validate() {
        eprintln!("Invalid configuration: {}", e);
        error!("Invalid configuration: {}", e);
        std::process::exit(1);
    }
    env::set_var(
        "IVNC_PROXY_PANEL_ENABLED",
        if config.ui_features.proxy_enabled {
            "1"
        } else {
            "0"
        },
    );
    let width = config.display.width;
    let height = config.display.height;
    info!("Display: {}x{}", width, height);
    info!(
        "Codec: {:?}, Bitrate: {} kbps",
        config.webrtc.video_codec, config.webrtc.video_bitrate
    );

    let runtime_settings = Arc::new(runtime_settings::RuntimeSettings::new(&config));
    let (input_tx, input_rx) = mpsc::unbounded_channel::<InputEventData>();
    let ui_config = config::ui::UiConfig::from_env(&config);

    if let Err(e) = display::initial_size(&config, &ui_config) {
        error!("Invalid desktop dimensions: {e}");
        std::process::exit(1);
    }
    let saved_runtime = console_config::load().runtime;
    if let Some(v) = saved_runtime.target_fps {
        runtime_settings.set_target_fps(v);
    }
    if let Some(v) = saved_runtime.video_bitrate_kbps {
        runtime_settings.set_video_bitrate_kbps(v);
    }
    if let Some(v) = saved_runtime.audio_bitrate {
        runtime_settings.set_audio_bitrate(v);
    }
    if let Some(v) = saved_runtime.keyframe_interval {
        runtime_settings.set_keyframe_interval(v);
    }
    if let Some(v) = saved_runtime.binary_clipboard_enabled {
        runtime_settings
            .apply_settings_json(&serde_json::json!({"enable_binary_clipboard":v}).to_string());
    }
    let shared_state = Arc::new(web::SharedState::new(
        config.clone(),
        ui_config,
        input_tx.clone(),
        runtime_settings.clone(),
        args.proxy_ip,
        args.proxy_autostart,
    ));

    if let Err(e) = run(config, shared_state, runtime_settings, input_rx) {
        eprintln!("Fatal error: {}", e);
        error!("Fatal error: {}", e);
        std::process::exit(1);
    }
}

fn run(
    config: Config,
    shared_state: Arc<web::SharedState>,
    runtime_settings: Arc<runtime_settings::RuntimeSettings>,
    mut input_rx: mpsc::UnboundedReceiver<InputEventData>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = shared_state.display_size();
    let (encoded_width, encoded_height, initial_fps, initial_bitrate) =
        shared_state.video_parameters(width, height);
    shared_state
        .display
        .set_encoded(display::Size::new(encoded_width, encoded_height));
    let running = Arc::new(AtomicBool::new(true));
    shared_state.set_shutdown_flag(running.clone());

    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::wayland_server::Display;

    // All env::set_var calls below happen before any threads are spawned
    // (tokio runtime is created later). This is important because set_var
    // is not thread-safe.

    // Ensure XDG_RUNTIME_DIR is set (required for Wayland socket)
    if env::var("XDG_RUNTIME_DIR").is_err() {
        let dir = format!("/run/user/{}", unsafe { libc::getuid() });
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!(
            "Cannot create {dir}: {e}. Set XDG_RUNTIME_DIR to a private directory owned by the running user (mode 0700), or use a service manager RuntimeDirectory."
        ))?;
        // Existing directories are validated below, never chmod'ed or taken over.
        env::set_var("XDG_RUNTIME_DIR", &dir);
        info!("Set XDG_RUNTIME_DIR={}", dir);
    }

    apps::launch::validate_runtime(std::path::Path::new(&env::var("XDG_RUNTIME_DIR")?))
        .map_err(String::from)?;

    // Clean up wayland sockets: kill non-ivnc processes occupying them
    if let Ok(xdg_dir) = env::var("XDG_RUNTIME_DIR") {
        for i in 0..3 {
            let sock = format!("{}/wayland-{}", xdg_dir, i);
            let lock = format!("{}.lock", sock);
            if !std::path::Path::new(&sock).exists() {
                continue;
            }
            if std::os::unix::net::UnixStream::connect(&sock).is_err() {
                // Stale socket, just remove
                std::fs::remove_file(&sock).ok();
                std::fs::remove_file(&lock).ok();
                continue;
            }
            // Socket is alive — find and kill the non-ivnc listener
            if let Ok(output) = std::process::Command::new("fuser").arg(&sock).output() {
                let pids_str = String::from_utf8_lossy(&output.stdout);
                for token in pids_str.split_whitespace() {
                    let pid: i32 = match token.trim().parse() {
                        Ok(p) if p > 1 => p,
                        _ => continue,
                    };
                    // Check if it's our own process
                    if pid == std::process::id() as i32 {
                        continue;
                    }
                    // Read process name
                    let comm =
                        std::fs::read_to_string(format!("/proc/{}/comm", pid)).unwrap_or_default();
                    if comm.trim() == "ivnc" {
                        continue;
                    }
                    warn!(
                        "Terminating non-ivnc process {} ({}) occupying {}",
                        pid,
                        comm.trim(),
                        sock
                    );
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    // If still alive after SIGTERM, force kill
                    if unsafe { libc::kill(pid, 0) } == 0 {
                        warn!(
                            "Process {} did not exit after SIGTERM, sending SIGKILL",
                            pid
                        );
                        unsafe {
                            libc::kill(pid, libc::SIGKILL);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
                std::fs::remove_file(&sock).ok();
                std::fs::remove_file(&lock).ok();
            }
        }
    }

    let mut event_loop: EventLoop<Compositor> = EventLoop::try_new()?;
    let display: Display<Compositor> = Display::new()?;
    let mut comp = Compositor::new(&mut event_loop, display);

    let mut backend = HeadlessBackend::new(width, height)?;
    let _output_global = backend
        .output()
        .create_global::<Compositor>(&comp.display_handle);
    comp.space.map_output(backend.output(), (0, 0));

    let socket_name = comp.socket_name.clone();
    env::set_var("WAYLAND_DISPLAY", &socket_name);
    // Disable GTK3 CSD via gtk3-nocsd (process-level env only)
    env::set_var("GTK_CSD", "0");
    let nocsd_lib = "/lib/x86_64-linux-gnu/libgtk3-nocsd.so.0";
    if std::path::Path::new(nocsd_lib).exists() {
        let existing = env::var("LD_PRELOAD").unwrap_or_default();
        if existing.is_empty() {
            env::set_var("LD_PRELOAD", nocsd_lib);
        } else {
            env::set_var("LD_PRELOAD", format!("{}:{}", existing, nocsd_lib));
        }
        info!("gtk3-nocsd enabled via LD_PRELOAD");
    }
    // Set up GTK CSS to hide headerbars on fullscreen windows only.
    // Dialogs are excluded so their controls (Open/Cancel buttons) stay visible.
    setup_gtk_css_env();
    write_env_file();
    info!("Wayland socket: {:?}", socket_name);

    // GStreamer pipeline
    let pipeline_config = PipelineConfig {
        source_width: width,
        source_height: height,
        width: encoded_width,
        height: encoded_height,
        framerate: initial_fps,
        codec: config.webrtc.video_codec,
        bitrate: initial_bitrate,
        hardware_encoder: config.webrtc.hardware_encoder,
        keyframe_interval: config.webrtc.keyframe_interval,
        latency_ms: config.webrtc.pipeline_latency_ms,
    };
    let mut pipeline = gstreamer::VideoPipeline::new(pipeline_config)?;
    pipeline.start()?;
    info!(
        "GStreamer pipeline started (encoder: {})",
        pipeline.encoder_name()
    );

    // Tokio runtime for async services
    let tokio_rt = tokio::runtime::Runtime::new()?;
    {
        let r = running.clone();
        tokio_rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("Ctrl+C received, shutting down gracefully");
                r.store(false, Ordering::SeqCst);
            }
        });
    }
    #[cfg(unix)]
    {
        let r = running.clone();
        tokio_rt.spawn(async move {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    sigterm.recv().await;
                    info!("SIGTERM received, shutting down gracefully");
                    r.store(false, Ordering::SeqCst);
                }
                Err(err) => warn!("Failed to install SIGTERM handler: {}", err),
            }
        });
    }
    {
        let st = shared_state.clone();
        let r = running.clone();
        let c = config.clone();
        let rs = runtime_settings.clone();
        tokio_rt.spawn(async move {
            let result = run_async_services(c, st, rs, r).await;
            if let Err(e) = result {
                error!("Async services error: {}", e);
            }
        });
    }

    // Ensure PulseAudio is running (needed for audio capture)
    #[cfg(feature = "pulseaudio")]
    if config.audio.enabled && !remote::worker::is_worker() {
        ensure_pulseaudio();
    }

    remote::worker::start_application()?;

    // Audio capture thread
    if config.audio.enabled {
        info!(
            "Starting audio capture thread (rate={} ch={} bitrate={})",
            config.audio.sample_rate, config.audio.channels, config.audio.bitrate
        );
        let r = running.clone();
        let ac = config.audio.clone();
        let (audio_tx, mut audio_rx) = mpsc::unbounded_channel();
        let st = shared_state.clone();
        tokio_rt.spawn(async move {
            while let Some(pkt) = audio_rx.recv().await {
                st.broadcast_audio(pkt);
            }
        });
        std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                info!("Audio capture thread started");
                let rt_audio = RuntimeAudioConfig {
                    sample_rate: ac.sample_rate,
                    channels: ac.channels,
                    bitrate: ac.bitrate,
                };
                match run_audio_capture(rt_audio, audio_tx, r) {
                    Ok(()) => info!("Audio capture thread exited normally"),
                    Err(e) => warn!("Audio capture ended with error: {}", e),
                }
            })?;
    } else {
        info!("Audio capture disabled in config");
    }

    // Main compositor loop
    let target_fps = shared_state.config.encoding.target_fps.max(1);
    let mut last_frame = Instant::now();
    let mut last_stats = Instant::now();
    let mut last_process_cpu_sample = read_process_cpu_ticks().map(|ticks| (Instant::now(), ticks));
    let mut frame_count: u64 = 0;
    let mut byte_count: u64 = 0;

    let mut render_frames: u64 = 0;
    let mut rtp_packets: u64 = 0;
    let mut prev_window_count: usize = 0;
    let mut keyframe_buf: Vec<Vec<u8>> = Vec::new();
    let mut in_keyframe = false;
    let mut rtp_frame_buf: Vec<Vec<u8>> = Vec::new();
    let mut prev_rtp_ts: Option<u32> = None;
    let mut last_rtp_sample: Option<Instant> = None;
    let mut last_render = Instant::now();
    let mut prev_button_mask: u32 = 0;
    let (disp_w, disp_h) = shared_state.display_size();
    let mut prev_cursor_pos: (f64, f64) = (disp_w as f64 / 2.0, disp_h as f64 / 2.0);
    let mut prev_cursor_message: String = "default".to_string();
    let mut prev_taskbar_json: String = String::new();
    let mut prev_dc_open_count: u64 = 0;
    let mut prev_apps_generation: u64 = 0;
    let mut render_state = RenderState::Active;
    let mut last_visual_activity_at = Instant::now();
    let mut video_update_retry_at = Instant::now();
    info!("Compositor loop starting at {} fps", target_fps);

    while running.load(Ordering::Relaxed) {
        let dispatch_state = desired_render_state(&shared_state, last_visual_activity_at);
        let dispatch_timeout = if dispatch_state == RenderState::Active {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(50)
        };
        let binary_clipboard_enabled = shared_state.config.input.enable_clipboard
            && runtime_settings.binary_clipboard_enabled();
        if comp.binary_clipboard_enabled != binary_clipboard_enabled {
            comp.binary_clipboard_enabled = binary_clipboard_enabled;
            shared_state.send_text(clipboard::paste::capabilities_message(
                shared_state.config.input.enable_clipboard,
                binary_clipboard_enabled,
            ));
        }
        event_loop.dispatch(Some(dispatch_timeout), &mut comp)?;
        comp.space.refresh();
        comp.popups.cleanup();
        comp.display_handle.flush_clients().ok();

        if drain_input_events(
            &mut input_rx,
            &mut comp,
            &shared_state,
            &mut prev_button_mask,
            &mut prev_cursor_pos,
        ) {
            last_visual_activity_at = Instant::now();
        }
        comp.display_handle.flush_clients().ok(); // flush injected input events immediately

        if let Some((revision, mut contents)) = comp.read_clipboard() {
            if !runtime_settings.binary_clipboard_enabled() {
                contents.retain(|(mime, _)| mime == "text/plain");
            }
            if shared_state.config.input.enable_clipboard {
                shared_state.set_clipboard_formats(contents, revision);
            }
        }

        // Named and SHM cursor changes share the browser's cursor protocol.
        if comp.cursor_dirty {
            comp.cursor_dirty = false;
            let message = comp.cursor_message();
            if message != prev_cursor_message {
                shared_state.send_text(message.clone());
                prev_cursor_message = message;
                last_visual_activity_at = Instant::now();
            }
        }

        // Detect window changes and request keyframe so browsers can decode the new content
        let cur_window_count = comp.space.elements().count();
        if cur_window_count != prev_window_count {
            info!(
                "Window count changed: {} -> {}",
                prev_window_count, cur_window_count
            );
            prev_window_count = cur_window_count;
            backend.reset_damage();
            pipeline.request_keyframe();
            comp.needs_redraw = true;
            comp.taskbar_dirty = true;
            last_visual_activity_at = Instant::now();
        }

        // Force taskbar resend when a new DataChannel opens
        // (receiver_count increases at subscribe time, before the DC is ready,
        //  so we use the datachannel_open_count which bumps on ChannelOpen)
        let cur_dc_open = shared_state.datachannel_open_count.load(Ordering::Relaxed);
        if cur_dc_open > prev_dc_open_count {
            prev_taskbar_json.clear();
            comp.taskbar_dirty = true;
            shared_state.send_text(prev_cursor_message.clone());
            last_visual_activity_at = Instant::now();
        }
        prev_dc_open_count = cur_dc_open;

        // Resend the taskbar when its application tabs change
        let cur_apps_generation = shared_state.apps_generation.load(Ordering::Relaxed);
        if cur_apps_generation != prev_apps_generation {
            prev_apps_generation = cur_apps_generation;
            comp.taskbar_dirty = true;
            last_visual_activity_at = Instant::now();
        }

        // Broadcast taskbar window list to frontend when dirty
        if comp.taskbar_dirty {
            comp.taskbar_dirty = false;
            let focused_wl = comp.seat.get_keyboard().and_then(|kb| kb.current_focus());
            let mut windows_json = Vec::new();
            let snapshot = shared_state.taskbar_apps();
            let mut window_pids = std::collections::HashSet::new();
            for (idx, wl_surface) in comp.window_registry.iter().enumerate() {
                if comp.window_for_surface(wl_surface).is_none() {
                    continue;
                }
                let is_focused = focused_wl
                    .as_ref()
                    .map(|f| comp.root_window_surface(f) == *wl_surface)
                    .unwrap_or(false);
                let (title, app_id) =
                    smithay::wayland::compositor::with_states(wl_surface, |states| {
                        let data = states
                            .data_map
                            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                            .unwrap()
                            .lock()
                            .unwrap();
                        (
                            data.title.clone().unwrap_or_default(),
                            data.app_id.clone().unwrap_or_default(),
                        )
                    });
                if is_focused {
                    comp.focused_surface_id = Some(idx as u32);
                }
                let pid = wl_surface
                    .client()
                    .and_then(|c| c.get_credentials(&comp.display_handle).ok())
                    .map(|c| c.pid);
                window_pids.extend(pid);
                let managed_app_id = pid.and_then(|pid| snapshot.window_apps.get(&pid));
                let name = managed_app_id.and_then(|id| snapshot.names.get(id));
                windows_json.push(serde_json::json!({
                    "id": idx,
                    "window_key": comp.window_keys.get(&wl_surface.id()).map(uuid::Uuid::to_string),
                    "title": title,
                    "app_id": app_id,
                    "display_name": resolve_display_name(&app_id),
                    "focused": is_focused,
                    "minimized": comp.minimized_windows.contains_key(&wl_surface.id()),
                    "managed_app_id": managed_app_id,
                    "managed_app_name": name.map(|n| &n.name),
                    "name_revision": name.map(|n| n.name_revision),
                    "can_rename": name.is_some_and(|n| n.can_rename),
                }));
            }
            // Runtimes have no apps manager, so their list of tabs stays empty.
            if !window_pids.is_empty() {
                remote::worker::window_seen();
            }
            shared_state.set_window_pids(window_pids);
            let json = web::shared::taskbar_json(windows_json, &snapshot);
            if json != prev_taskbar_json {
                prev_taskbar_json = json.clone();
                let msg = format!("taskbar,{}", json);
                info!("Taskbar broadcast: {}", msg);
                shared_state.send_text(msg);
                // Cache for the taskbar window list
                *shared_state.last_taskbar_json.lock().unwrap() = Some(json);
                last_visual_activity_at = Instant::now();
            }
        }

        let resize = shared_state.display.take_request();
        let old_desktop = shared_state.display.snapshot().desktop;
        let target = resize.as_ref().map(|r| r.size).unwrap_or(old_desktop);
        let (w, h, fps, bitrate) = shared_state.video_parameters(target.width, target.height);
        let current = pipeline.config();
        let rebuild = (target.width, target.height, w, h, fps, bitrate)
            != (
                current.source_width,
                current.source_height,
                current.width,
                current.height,
                current.framerate,
                current.bitrate,
            );
        let mut display_changed = false;
        if resize.is_some() || (rebuild && Instant::now() >= video_update_retry_at) {
            video_update_retry_at = Instant::now() + Duration::from_secs(3);
            let result = (|| -> Result<(), String> {
                if !rebuild {
                    return console_config::update(|c| c.display = Some(target));
                }
                let prepared = if target != old_desktop {
                    Some(backend.prepare_resize(target.width, target.height)?)
                } else {
                    None
                };
                let new_pipeline = gstreamer::VideoPipeline::new(PipelineConfig {
                    source_width: target.width,
                    source_height: target.height,
                    width: w,
                    height: h,
                    framerate: fps,
                    bitrate,
                    codec: config.webrtc.video_codec,
                    hardware_encoder: config.webrtc.hardware_encoder,
                    keyframe_interval: runtime_settings.keyframe_interval(),
                    latency_ms: config.webrtc.pipeline_latency_ms,
                })
                .map_err(|e| e.to_string())?;
                let _ = pipeline.stop();
                if let Err(e) = new_pipeline.start() {
                    let _ = new_pipeline.stop();
                    let _ = pipeline.start();
                    return Err(e.to_string());
                }
                let rollback = prepared.map(|p| backend.apply_resize(p));
                if resize.is_some() {
                    if let Err(e) = console_config::update(|c| c.display = Some(target)) {
                        if let Some(old) = rollback {
                            backend.apply_resize(old);
                        }
                        let _ = new_pipeline.stop();
                        let _ = pipeline.start();
                        return Err(format!("Cannot save desktop size: {e}"));
                    }
                }
                pipeline = new_pipeline;
                if target != old_desktop {
                    comp.resize_windows(target.width, target.height);
                    last_render = Instant::now() - Duration::from_secs(1);
                    let _ = shared_state.input_sender.send(InputEventData {
                        event_type: InputEvent::KeyboardReset,
                        ..Default::default()
                    });
                }
                if resize.is_none() {
                    display_changed |= shared_state.display.set_encoded(display::Size::new(w, h));
                }
                comp.needs_redraw = true;
                comp.taskbar_dirty = true;
                // Never reuse packets or cached frames from the previous encoder.
                rtp_frame_buf.clear();
                keyframe_buf.clear();
                prev_rtp_ts = None;
                last_rtp_sample = None;
                in_keyframe = false;
                shared_state.set_keyframe_cache(Vec::new());
                pipeline.request_keyframe();
                video_update_retry_at = Instant::now();
                info!(
                    "Desktop {}x{}, video {}x{} at {} fps, {} kbps",
                    target.width, target.height, w, h, fps, bitrate
                );
                Ok(())
            })();
            if let Err(ref e) = result {
                error!("Display/video update failed: {e}");
            }
            if let Some(request) = resize {
                shared_state
                    .display
                    .complete(request, result, display::Size::new(w, h));
                display_changed = true;
            }
            last_visual_activity_at = Instant::now();
        }
        if display_changed {
            shared_state.send_text(shared_state.display_message());
        }

        if comp.needs_redraw {
            last_visual_activity_at = Instant::now();
        }
        apply_runtime_settings(&runtime_settings, &pipeline);

        let next_render_state = desired_render_state(&shared_state, last_visual_activity_at);
        if next_render_state != render_state {
            handle_render_state_transition(
                render_state,
                next_render_state,
                &mut backend,
                &mut comp,
                &pipeline,
                &mut rtp_frame_buf,
                &mut prev_rtp_ts,
                &mut last_rtp_sample,
                &mut in_keyframe,
                &mut keyframe_buf,
                &mut last_frame,
                &mut last_render,
            );
            render_state = next_render_state;
            shared_state.set_render_state(render_state);
        }

        if render_state == RenderState::Active {
            let frame_duration =
                Duration::from_micros(1_000_000 / u64::from(pipeline.config().framerate.max(1)));
            // Send frame callbacks BEFORE sleep so clients have the full
            // frame period to prepare and commit their next buffer.
            backend.send_frame_callbacks(&comp);
            comp.display_handle.flush_clients().ok();

            // Frame timing — clients are working in parallel during this sleep
            let elapsed = last_frame.elapsed();
            if elapsed < frame_duration {
                std::thread::sleep(frame_duration - elapsed);
            }
            last_frame = Instant::now();

            // Quick dispatch to pick up commits that arrived during sleep
            event_loop.dispatch(Some(Duration::ZERO), &mut comp)?;
            comp.display_handle.flush_clients().ok();

            // Render + encode if any client committed new content.
            // Also force periodic renders when sessions are active to ensure
            // the browser always has decodable video frames.
            if !comp.needs_redraw && last_render.elapsed() >= Duration::from_secs(1) {
                comp.needs_redraw = true;
            }
            if comp.needs_redraw {
                comp.needs_redraw = false;
                match backend.render_frame(&mut comp) {
                    Some(pixels) => {
                        render_frames += 1;
                        last_render = Instant::now();
                        if let Err(e) = pipeline.push_frame(&pixels) {
                            warn!("Failed to push frame: {}", e);
                            continue;
                        }
                        frame_count += 1;
                        byte_count += pixels.len() as u64;
                    }
                    None => {
                        warn!(
                            "render_frame returned None (windows={})",
                            comp.space.elements().count()
                        );
                    }
                }
            }
        }

        if render_state == RenderState::Active {
            pull_and_broadcast_rtp(
                &pipeline,
                &shared_state,
                &mut rtp_packets,
                &mut keyframe_buf,
                &mut in_keyframe,
                &mut rtp_frame_buf,
                &mut prev_rtp_ts,
                &mut last_rtp_sample,
            );
        }

        if shared_state.take_keyframe_request() {
            pipeline.request_keyframe();
        }

        if last_stats.elapsed() >= Duration::from_secs(1) {
            let secs = last_stats.elapsed().as_secs_f64();
            let windows = comp.space.elements().count();
            // Application workers log to a per-application file; keep it to events.
            log::log!(
                if remote::worker::is_worker() { log::Level::Debug } else { log::Level::Info },
                "Loop stats: idle={}, viewers={}, rtp_receivers={}, windows={}, rendered={}, pushed={}, rtp_pkts={}, secs={:.1}",
                render_state.as_str(),
                shared_state.webrtc_sessions(),
                shared_state.rtp_receiver_count(),
                windows,
                render_frames,
                frame_count,
                rtp_packets,
                secs
            );
            {
                let mut stats = shared_state.stats.lock().unwrap();
                stats.fps = frame_count as f64 / secs;
                stats.bandwidth = (byte_count as f64 * 8.0 / secs) as u64;
                stats.total_frames += frame_count;
                stats.total_bytes += byte_count;
                if let Some((sample_at, ticks)) =
                    read_process_cpu_ticks().map(|ticks| (Instant::now(), ticks))
                {
                    if let Some((prev_at, prev_ticks)) = last_process_cpu_sample {
                        let elapsed = sample_at.duration_since(prev_at).as_secs_f64();
                        if elapsed > 0.0 && ticks >= prev_ticks {
                            let hz = clock_ticks_per_second();
                            stats.cpu_percent =
                                ((ticks - prev_ticks) as f64 / hz / elapsed) * 100.0;
                        }
                    }
                    last_process_cpu_sample = Some((sample_at, ticks));
                }
                if let Some(mem_used) = read_process_rss_bytes() {
                    stats.mem_used = mem_used;
                }
            }
            shared_state.send_text(format!("stats,{}", shared_state.stats_json()));
            // Re-broadcast cursor state so newly connected sessions get it
            shared_state.send_text(prev_cursor_message.clone());
            render_frames = 0;
            frame_count = 0;
            byte_count = 0;
            rtp_packets = 0;
            last_stats = Instant::now();
        }
    }

    info!("Shutting down...");
    running.store(false, Ordering::SeqCst);
    shared_state.proxy_panel.begin_shutdown();
    if let Some(apps_state) = shared_state.apps_state().cloned() {
        if let Err(err) = tokio_rt.block_on(apps_state.shutdown_running_apps_with_state()) {
            warn!(
                "Failed to save and stop running apps during shutdown: {}",
                err
            );
        }
    }
    tokio_rt.block_on(shared_state.proxy_panel.shutdown());
    let _ = pipeline.stop();
    tokio_rt.shutdown_timeout(Duration::from_secs(3));
    cleanup_env_file();
    cleanup_wayland_socket(&socket_name);
    info!("ivnc stopped");
    Ok(())
}

fn drain_input_events(
    input_rx: &mut mpsc::UnboundedReceiver<InputEventData>,
    state: &mut Compositor,
    shared: &Arc<web::SharedState>,
    prev_button_mask: &mut u32,
    prev_cursor_pos: &mut (f64, f64),
) -> bool {
    use smithay::utils::SERIAL_COUNTER;

    let mut had_input = false;
    while let Some(mut ev) = {
        if state.ime_paste_pending && state.clipboard_paste.busy() {
            return had_input;
        }
        state.ime_paste_pending = false;
        state
            .deferred_input
            .pop_front()
            .or_else(|| input_rx.try_recv().ok())
    } {
        // Events can have queued while an administrator changed the desktop.
        if ev
            .geometry_revision
            .is_some_and(|r| r != shared.display.snapshot().revision)
        {
            continue;
        }
        had_input = true;
        let serial = SERIAL_COUNTER.next_serial();
        // Use monotonic clock for Wayland event timestamps (milliseconds).
        // The frontend doesn't send timestamps for keyboard events, so
        // ev.timestamp is 0 — Chromium may discard events with time=0.
        let time = (state.start_time.elapsed().as_millis() & 0xFFFFFFFF) as u32;

        match ev.event_type {
            InputEvent::MouseMove => {
                let (mut x, mut y) = if ev.text == "relative" {
                    (
                        prev_cursor_pos.0 + ev.mouse_x as f64,
                        prev_cursor_pos.1 + ev.mouse_y as f64,
                    )
                } else {
                    (ev.mouse_x as f64, ev.mouse_y as f64)
                };
                let (disp_w, disp_h) = shared.display_size();
                x = x.clamp(0.0, disp_w.saturating_sub(1) as f64);
                y = y.clamp(0.0, disp_h.saturating_sub(1) as f64);
                let ptr = state.seat.get_pointer().unwrap();
                let old = ptr.current_location();
                let delta = if ev.text == "relative" {
                    (ev.mouse_x as f64, ev.mouse_y as f64).into()
                } else {
                    smithay::utils::Point::from((x, y)) - old
                };
                let pos = state.constrained_pointer_position((x, y).into());
                *prev_cursor_pos = (pos.x, pos.y);
                let under = state.surface_under(pos);
                ptr.relative_motion(
                    state,
                    under.clone(),
                    &smithay::input::pointer::RelativeMotionEvent {
                        delta,
                        delta_unaccel: delta,
                        utime: state.start_time.elapsed().as_micros() as u64,
                    },
                );
                // The lock protocol forbids wl_pointer.motion while locked.
                // Sending unchanged absolute coordinates resets Chromium's
                // relative motion accumulator and discards the raw deltas.
                if !state.pointer_locked() {
                    ptr.motion(
                        state,
                        under.clone(),
                        &smithay::input::pointer::MotionEvent {
                            location: pos,
                            serial,
                            time,
                        },
                    );
                }
                ptr.frame(state);
                state.activate_pointer_constraint();

                // Re-send keyboard focus after the first pointer enter.
                // Chromium's Ozone/Wayland layer ignores keyboard events received
                // before wl_pointer.enter, so we re-send wl_keyboard.enter once
                // the pointer has entered the surface.
                if state.kbd_focus_needs_reenter && under.is_some() {
                    let keyboard = state.seat.get_keyboard().unwrap();
                    if let Some(focus) = keyboard.current_focus() {
                        let reenter_serial = SERIAL_COUNTER.next_serial();
                        info!("Re-sending keyboard focus after first pointer enter");
                        keyboard.set_focus(state, None, reenter_serial);
                        let reenter_serial2 = SERIAL_COUNTER.next_serial();
                        keyboard.set_focus(state, Some(focus), reenter_serial2);
                    }
                    state.kbd_focus_needs_reenter = false;
                }

                // Synthesize button events from buttonMask changes.
                // The frontend sends m,x,y,buttonMask,0 — button state is
                // encoded in the mask, not as separate b,button,pressed messages.
                let new_mask = ev.button_mask;
                if new_mask != *prev_button_mask {
                    info!(
                        "ButtonMask changed: {} -> {} at ({},{})",
                        *prev_button_mask, new_mask, ev.mouse_x, ev.mouse_y
                    );
                    let changed = new_mask ^ *prev_button_mask;
                    for bit in 0..5u8 {
                        if changed & (1 << bit) != 0 {
                            let pressed = new_mask & (1 << bit) != 0;
                            let synth = InputEventData {
                                event_type: InputEvent::MouseButton,
                                mouse_x: x as i32,
                                mouse_y: y as i32,
                                mouse_button: bit,
                                button_pressed: pressed,
                                ..Default::default()
                            };
                            let btn_serial = SERIAL_COUNTER.next_serial();
                            inject_button(state, &synth, btn_serial, time);
                        }
                    }
                    *prev_button_mask = new_mask;
                }
            }
            InputEvent::MouseButton => {
                inject_button(state, &ev, serial, time);
            }
            InputEvent::MouseWheel => {
                inject_scroll(state, &ev, time);
            }
            InputEvent::Keyboard => {
                inject_key(state, &ev, serial, time);
            }
            InputEvent::KeyboardReset => {
                state.release_pointer_constraint();
                // Release every pressed key and pointer button to clear stuck state.
                let keyboard = state.seat.get_keyboard().unwrap();
                let pressed = keyboard.pressed_keys();
                for kc in pressed {
                    let s = smithay::utils::SERIAL_COUNTER.next_serial();
                    keyboard.input::<(), _>(
                        state,
                        kc,
                        smithay::backend::input::KeyState::Released,
                        s,
                        time,
                        |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                    );
                }
                for bit in 0..5u8 {
                    if *prev_button_mask & (1 << bit) != 0 {
                        inject_button(
                            state,
                            &InputEventData {
                                event_type: InputEvent::MouseButton,
                                mouse_button: bit,
                                button_pressed: false,
                                ..Default::default()
                            },
                            SERIAL_COUNTER.next_serial(),
                            time,
                        );
                    }
                }
                *prev_button_mask = 0;
            }
            InputEvent::Ping => {
                shared.send_text("pong".to_string());
            }
            InputEvent::Clipboard | InputEvent::ClipboardBinary => {
                if shared.config.input.enable_clipboard {
                    if state.clipboard_paste.busy() {
                        state.deferred_input.push_front(ev);
                        break;
                    }
                    let binary = ev.event_type == InputEvent::ClipboardBinary;
                    if binary && !shared.runtime_settings.binary_clipboard_enabled() {
                        continue;
                    }
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&ev.text) {
                        if bytes.len() > clipboard::MAX_CLIPBOARD_BYTES {
                            continue;
                        }
                        if binary {
                            let valid = match ev.mime_type.as_str() {
                                "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                                "text/html" => std::str::from_utf8(&bytes).is_ok(),
                                _ => false,
                            };
                            if valid {
                                let formats = [clipboard::paste::FormatSpec {
                                    mime: ev.mime_type,
                                    size: bytes.len(),
                                }];
                                state.install_clipboard_formats(Arc::from(bytes), &formats, None);
                            }
                        } else if std::str::from_utf8(&bytes).is_ok() {
                            let revision = state.install_clipboard(Arc::from(bytes), None);
                            shared.set_clipboard(ev.text, revision, "browser");
                        }
                    }
                }
            }
            InputEvent::PasteStart => {
                if let Some(command) = ev.paste {
                    state.reserve_paste(&command);
                }
            }
            InputEvent::Paste => {
                if let Some(command) = ev.paste {
                    if !command.formats.is_empty()
                        && !shared.runtime_settings.binary_clipboard_enabled()
                    {
                        command.respond("disabled", None);
                        continue;
                    }
                    match state.paste(&command) {
                        Ok(revision) => {
                            if let Some(text) = command.plain_text() {
                                shared.set_clipboard(
                                    base64::engine::general_purpose::STANDARD.encode(text),
                                    revision,
                                    "browser",
                                );
                            }
                        }
                        Err(reason) => command.respond(reason, None),
                    }
                }
            }
            InputEvent::TextInput => {
                if !ev.text.is_empty() && ev.paste.is_none() {
                    if let Some(surface) =
                        state.seat.get_keyboard().and_then(|kb| kb.current_focus())
                    {
                        state.last_user_serial = Some((serial, Instant::now(), surface));
                    }
                }
                if !ev.text.is_empty() && !state.inject_committed_text(&ev.text, &mut ev.paste) {
                    state.deferred_input.push_front(ev);
                    break;
                }
            }
            InputEvent::WindowFocus => {
                if let Some(surface) = state.window_registry.get(ev.window_id as usize).cloned() {
                    state.activate_window(&surface);
                }
            }
            InputEvent::WindowFocusStable => {
                if let Some(key) = ev.window_key {
                    if let Some(surface) = state
                        .window_registry
                        .iter()
                        .find(|surface| state.window_keys.get(&surface.id()) == Some(&key))
                        .cloned()
                    {
                        state.activate_window(&surface);
                    }
                }
            }
            InputEvent::WindowClose => {
                if let Some(surface) = state.window_registry.get(ev.window_id as usize) {
                    if let Some(window) = state.window_for_surface(surface) {
                        // Focus moves only when the client actually destroys the
                        // window: Electron may cancel close to ask about changes.
                        window.toplevel().unwrap().send_close();
                    }
                }
            }
        }
    }
    had_input
}

fn inject_button(
    state: &mut Compositor,
    ev: &InputEventData,
    serial: smithay::utils::Serial,
    time: u32,
) {
    let button = match ev.mouse_button {
        0 => 0x110u32,
        1 => 0x112,
        2 => 0x111,
        b => 0x110 + b as u32,
    };
    let btn_state = if ev.button_pressed {
        smithay::backend::input::ButtonState::Pressed
    } else {
        smithay::backend::input::ButtonState::Released
    };

    // On press, focus the toplevel window under the pointer so it receives keyboard input.
    // We must use the toplevel's wl_surface (not a subsurface from surface_under()),
    // because Chromium routes keyboard events based on which wl_surface has keyboard focus.
    // Using a subsurface would cause Chromium to ignore key events entirely.
    if ev.button_pressed {
        let pos = state.seat.get_pointer().unwrap().current_location();
        if let Some((window, _)) = state.space.element_under(pos) {
            if let Some(toplevel) = window.toplevel() {
                let wl_surface = toplevel.wl_surface().clone();
                let keyboard = state.seat.get_keyboard().unwrap();
                if let Some(window) = state.window_for_surface(&wl_surface) {
                    state.space.raise_element(&window, true);
                }
                keyboard.set_focus(state, Some(wl_surface.clone()), serial);
                state.last_user_serial = Some((serial, Instant::now(), wl_surface));
            }
        }
    }

    let ptr = state.seat.get_pointer().unwrap();
    ptr.button(
        state,
        &smithay::input::pointer::ButtonEvent {
            button,
            state: btn_state,
            serial,
            time,
        },
    );
    ptr.frame(state);
}

fn inject_scroll(state: &mut Compositor, ev: &InputEventData, time: u32) {
    use smithay::backend::input::Axis;
    use smithay::input::pointer::AxisFrame;
    let ptr = state.seat.get_pointer().unwrap();
    let mut frame = AxisFrame::new(time);
    if ev.wheel_delta_y != 0 {
        frame = frame.value(Axis::Vertical, ev.wheel_delta_y as f64);
    }
    if ev.wheel_delta_x != 0 {
        frame = frame.value(Axis::Horizontal, ev.wheel_delta_x as f64);
    }
    ptr.axis(state, frame);
    ptr.frame(state);
}

fn inject_key(
    state: &mut Compositor,
    ev: &InputEventData,
    serial: smithay::utils::Serial,
    time: u32,
) {
    use smithay::input::keyboard::{FilterResult, Keycode};
    let keyboard = state.seat.get_keyboard().unwrap();
    let key_state = if ev.key_pressed {
        smithay::backend::input::KeyState::Pressed
    } else {
        smithay::backend::input::KeyState::Released
    };

    // Frontend sends X11 keysyms; smithay expects xkb keycodes (evdev + 8).
    // Use a lookup table for the most common keysyms.
    let keycode = match keysym_to_keycode(ev.keysym) {
        Some(code) => code,
        None => {
            warn!("Unknown keysym 0x{:x}; dropping key event", ev.keysym);
            return;
        }
    };
    if ev.key_pressed {
        if let Some(surface) = keyboard.current_focus() {
            state.last_user_serial = Some((serial, Instant::now(), surface));
        }
    }
    let has_focus = keyboard.current_focus().is_some();
    info!(
        "inject_key: keysym=0x{:x} keycode={} pressed={} has_focus={}",
        ev.keysym, keycode, ev.key_pressed, has_focus
    );
    keyboard.input::<(), _>(
        state,
        Keycode::from(keycode),
        key_state,
        serial,
        time,
        |_, _, _| FilterResult::Forward,
    );
}

/// Convert X11 keysym to xkb keycode (evdev keycode + 8).
fn keysym_to_keycode(keysym: u32) -> Option<u32> {
    match keysym {
        // Letters (a-z / A-Z)
        0x61 | 0x41 => 38,
        0x62 | 0x42 => 56,
        0x63 | 0x43 => 54,
        0x64 | 0x44 => 40,
        0x65 | 0x45 => 26,
        0x66 | 0x46 => 41,
        0x67 | 0x47 => 42,
        0x68 | 0x48 => 43,
        0x69 | 0x49 => 31,
        0x6a | 0x4a => 44,
        0x6b | 0x4b => 45,
        0x6c | 0x4c => 46,
        0x6d | 0x4d => 58,
        0x6e | 0x4e => 57,
        0x6f | 0x4f => 32,
        0x70 | 0x50 => 33,
        0x71 | 0x51 => 24,
        0x72 | 0x52 => 27,
        0x73 | 0x53 => 39,
        0x74 | 0x54 => 28,
        0x75 | 0x55 => 30,
        0x76 | 0x56 => 55,
        0x77 | 0x57 => 25,
        0x78 | 0x58 => 53,
        0x79 | 0x59 => 29,
        0x7a | 0x5a => 52,
        // Digits 0-9 and shifted symbols on same keys
        0x30 | 0x29 => 19,
        0x31 | 0x21 => 10,
        0x32 | 0x40 => 11,
        0x33 | 0x23 => 12,
        0x34 | 0x24 => 13,
        0x35 | 0x25 => 14,
        0x36 | 0x5e => 15,
        0x37 | 0x26 => 16,
        0x38 | 0x2a => 17,
        0x39 | 0x28 => 18,
        // Function keys F1-F12
        0xffbe => 67,
        0xffbf => 68,
        0xffc0 => 69,
        0xffc1 => 70,
        0xffc2 => 71,
        0xffc3 => 72,
        0xffc4 => 73,
        0xffc5 => 74,
        0xffc6 => 75,
        0xffc7 => 76,
        0xffc8 => 95,
        0xffc9 => 96,
        // Modifiers
        0xffe1 => 50,
        0xffe2 => 62, // Shift L/R
        0xffe3 => 37,
        0xffe4 => 105, // Control L/R
        0xffe9 => 64,
        0xffea => 108, // Alt L/R
        0xffeb => 133,
        0xffec => 134, // Super L/R
        0xffe5 => 66,  // Caps_Lock
        // Navigation
        0xff0d => 36,
        0xff1b => 9,
        0xff08 => 22,
        0xff09 => 23,
        0x20 => 65,
        0xffff => 119,
        0xff63 => 118,
        0xff50 => 110,
        0xff57 => 115,
        0xff55 => 112,
        0xff56 => 117,
        // Arrows
        0xff51 => 113,
        0xff52 => 111,
        0xff53 => 114,
        0xff54 => 116,
        // Symbols (key and shifted variant grouped)
        0x2d | 0x5f => 20, // minus / underscore
        0x3d | 0x2b => 21, // equal / plus
        0x5b | 0x7b => 34, // bracketleft / braceleft
        0x5d | 0x7d => 35, // bracketright / braceright
        0x5c | 0x7c => 51, // backslash / bar
        0x3b | 0x3a => 47, // semicolon / colon
        0x27 | 0x22 => 48, // apostrophe / quotedbl
        0x60 | 0x7e => 49, // grave / tilde
        0x2c | 0x3c => 59, // comma / less
        0x2e | 0x3e => 60, // period / greater
        0x2f | 0x3f => 61, // slash / question
        // Misc
        0xff13 => 127,
        0xff14 => 78,
        0xff61 => 107,
        0xff7f => 77,
        _ => {
            log::debug!("Unknown keysym 0x{:x}, no keycode mapping", keysym);
            return None;
        }
    }
    .into()
}

/// Check if an RTP packet contains an H.264 keyframe NAL unit.
fn is_h264_keyframe_packet(data: &[u8]) -> bool {
    let hdr_len = webrtc::media_track::rtp_util::header_length(data).unwrap_or(12);
    if data.len() <= hdr_len {
        return false;
    }
    let nal_type = data[hdr_len] & 0x1F;
    match nal_type {
        5 | 7 | 8 => true,
        24 => true,
        28 if data.len() > hdr_len + 1 => (data[hdr_len + 1] & 0x1F) == 5,
        _ => false,
    }
}

fn pull_and_broadcast_rtp(
    pipeline: &gstreamer::VideoPipeline,
    shared: &Arc<web::SharedState>,
    rtp_count: &mut u64,
    keyframe_buf: &mut Vec<Vec<u8>>,
    in_keyframe: &mut bool,
    frame_buf: &mut Vec<Vec<u8>>,
    prev_ts: &mut Option<u32>,
    last_sample: &mut Option<Instant>,
) {
    while let Some(sample) = pipeline.try_pull_sample() {
        if let Some(buffer) = sample.buffer() {
            let map = buffer.map_readable().unwrap();
            let data = map.as_slice().to_vec();

            let ts = webrtc::media_track::rtp_util::get_timestamp(&data).unwrap_or(0);

            // When timestamp changes, the previous frame is complete —
            // set marker bit on its last packet and flush.
            if let Some(prev) = *prev_ts {
                if ts != prev && !frame_buf.is_empty() {
                    flush_frame(frame_buf, shared, rtp_count, keyframe_buf, in_keyframe);
                }
            }
            *prev_ts = Some(ts);
            frame_buf.push(data);
            *last_sample = Some(Instant::now());
            let has_marker = frame_buf
                .last()
                .map(|pkt| pkt.len() >= 2 && (pkt[1] & 0x80) != 0)
                .unwrap_or(false);
            if has_marker {
                flush_frame(frame_buf, shared, rtp_count, keyframe_buf, in_keyframe);
            }
        }
    }

    // If no new packets arrived for a short window, flush the buffered frame
    // to avoid stalling when marker bits are missing.
    if !frame_buf.is_empty() {
        if let Some(ts) = last_sample {
            if ts.elapsed() >= Duration::from_millis(50) {
                flush_frame(frame_buf, shared, rtp_count, keyframe_buf, in_keyframe);
            }
        }
    }
}

/// Set the marker bit on the last packet in the frame buffer, then broadcast all packets.
fn flush_frame(
    frame_buf: &mut Vec<Vec<u8>>,
    shared: &Arc<web::SharedState>,
    rtp_count: &mut u64,
    keyframe_buf: &mut Vec<Vec<u8>>,
    in_keyframe: &mut bool,
) {
    // Set marker bit on the last packet of the frame
    if let Some(last) = frame_buf.last_mut() {
        if last.len() >= 2 {
            last[1] |= 0x80;
        }
    }

    for data in frame_buf.drain(..) {
        let is_kf = is_h264_keyframe_packet(&data);
        if is_kf && !*in_keyframe {
            keyframe_buf.clear();
            *in_keyframe = true;
        }
        if *in_keyframe {
            keyframe_buf.push(data.clone());
            let marker = data.len() >= 2 && (data[1] & 0x80) != 0;
            if marker {
                shared.set_keyframe_cache(keyframe_buf.clone());
                log::info!(
                    "Cached keyframe: {} pkts, {} bytes",
                    keyframe_buf.len(),
                    keyframe_buf.iter().map(|p| p.len()).sum::<usize>()
                );
                *in_keyframe = false;
            }
        }

        *rtp_count += 1;
        if *rtp_count <= 3 || *rtp_count % 500 == 0 {
            log::info!(
                "broadcast_rtp #{} receivers={}",
                *rtp_count,
                shared.rtp_receiver_count()
            );
        }
        shared.broadcast_rtp(data);
    }
}

fn apply_runtime_settings(
    rs: &Arc<runtime_settings::RuntimeSettings>,
    pipeline: &gstreamer::VideoPipeline,
) {
    if rs.take_keyframe_request() {
        pipeline.request_keyframe();
    }
    // Bitrate/FPS changes rebuild the pipeline above so its H.264 level and
    // negotiated receive envelope stay consistent with the actual stream.
    let new_ki = rs.keyframe_interval();
    if new_ki != pipeline.config().keyframe_interval {
        pipeline.set_keyframe_interval(new_ki);
    }
}

fn desired_render_state(
    shared: &web::SharedState,
    last_visual_activity_at: Instant,
) -> RenderState {
    let has_transport_viewers = shared.webrtc_sessions() > 0 || shared.rtp_receiver_count() > 0;
    if !has_transport_viewers {
        return RenderState::NoViewerIdle;
    }

    let viewers = shared.client_viewer_summary();
    if viewers.has_recent_client_wakeup {
        return RenderState::Active;
    }
    if viewers.has_connections && viewers.all_clients_inactive {
        return RenderState::NoViewerIdle;
    }

    if last_visual_activity_at.elapsed() >= VIEWER_QUIET_IDLE_AFTER {
        return RenderState::ViewerQuietIdle;
    }

    RenderState::Active
}

#[allow(clippy::too_many_arguments)]
fn handle_render_state_transition(
    previous: RenderState,
    next: RenderState,
    backend: &mut HeadlessBackend,
    comp: &mut Compositor,
    pipeline: &gstreamer::VideoPipeline,
    rtp_frame_buf: &mut Vec<Vec<u8>>,
    prev_rtp_ts: &mut Option<u32>,
    last_rtp_sample: &mut Option<Instant>,
    in_keyframe: &mut bool,
    keyframe_buf: &mut Vec<Vec<u8>>,
    last_frame: &mut Instant,
    last_render: &mut Instant,
) {
    match (previous, next) {
        (RenderState::Active, RenderState::ViewerQuietIdle) => {
            info!("Entering viewer quiet idle: no window activity");
        }
        (RenderState::Active, RenderState::NoViewerIdle) => {
            info!("Entering deep idle: no active viewers");
        }
        (_, RenderState::Active) => {
            info!("Leaving render idle: active viewer or window activity detected");
            backend.reset_damage();
            comp.needs_redraw = true;
            comp.taskbar_dirty = true;
            pipeline.request_keyframe();
            rtp_frame_buf.clear();
            *prev_rtp_ts = None;
            *last_rtp_sample = None;
            *in_keyframe = false;
            keyframe_buf.clear();
            *last_frame = Instant::now();
            *last_render = Instant::now() - Duration::from_secs(1);
        }
        (RenderState::ViewerQuietIdle, RenderState::NoViewerIdle) => {
            info!("Entering deep idle: all viewers inactive");
        }
        (RenderState::NoViewerIdle, RenderState::ViewerQuietIdle) => {
            info!("Leaving deep idle: viewer available, waiting for window activity");
            backend.reset_damage();
            comp.needs_redraw = true;
            comp.taskbar_dirty = true;
            pipeline.request_keyframe();
            rtp_frame_buf.clear();
            *prev_rtp_ts = None;
            *last_rtp_sample = None;
            *in_keyframe = false;
            keyframe_buf.clear();
            *last_frame = Instant::now();
            *last_render = Instant::now() - Duration::from_secs(1);
        }
        _ => {}
    }
}

fn read_process_cpu_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after_comm = stat.rsplit_once(") ")?.1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some(utime + stime)
}

fn read_process_rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let rss_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(rss_pages * page_size_bytes())
}

fn clock_ticks_per_second() -> f64 {
    static HZ: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *HZ.get_or_init(|| unsafe {
        let value = libc::sysconf(libc::_SC_CLK_TCK);
        if value > 0 {
            value as f64
        } else {
            100.0
        }
    })
}

fn page_size_bytes() -> u64 {
    static PAGE_SIZE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PAGE_SIZE.get_or_init(|| unsafe {
        let value = libc::sysconf(libc::_SC_PAGESIZE);
        if value > 0 {
            value as u64
        } else {
            4096
        }
    })
}

async fn run_async_services(
    config: Config,
    shared: Arc<web::SharedState>,
    runtime_settings: Arc<runtime_settings::RuntimeSettings>,
    _running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let upload_settings = file_upload::FileUploadSettings::from_config(&config);

    // Session manager (WebRTC)
    let session_manager = if config.webrtc.enabled {
        // Resolve a routable IP for the ICE-TCP candidate.
        // 0.0.0.0 is not valid in SDP — the browser needs an actual address.
        let bind_ip: std::net::IpAddr = config
            .http
            .host
            .parse()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        let candidate_ip = if bind_ip.is_unspecified() {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        } else {
            bind_ip
        };
        let listen_addr = std::net::SocketAddr::new(candidate_ip, config.http.port);
        info!("ICE-TCP candidate address: {}", listen_addr);
        let sm = SessionManager::new(
            config.webrtc.clone(),
            shared.input_sender.clone(),
            upload_settings,
            runtime_settings.clone(),
            shared.clone(),
            16,
            listen_addr,
        );
        Some(Arc::new(sm))
    } else {
        None
    };

    // Apps manager
    let apps_state = if remote::worker::is_worker() {
        None
    } else {
        match crate::apps::api::AppsState::new(shared.proxy_panel.clone()) {
            Ok(ps) => {
                info!("Apps manager initialized");
                let ps_arc = std::sync::Arc::new(ps);
                ps_arc.start_monitor();

                // Restore previously running apps after update
                let ps_clone = ps_arc.clone();
                tokio::spawn(async move {
                    if let Err(error) = ps_clone.start_builtin_apps().await {
                        log::warn!("Miao autostart failed: {}", error);
                    }
                    // Wait a bit for the system to stabilize
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    if let Err(e) = ps_clone.restore_running_state().await {
                        log::warn!("Failed to restore running apps: {}", e);
                    } else {
                        log::info!("Running apps restoration completed");
                    }
                    if let Err(e) = ps_clone.start_autostart_apps().await {
                        log::warn!("Failed to start autostart apps: {}", e);
                    } else {
                        log::info!("Autostart apps initialization completed");
                    }
                });

                Some(ps_arc)
            }
            Err(e) => {
                warn!("Failed to init Apps manager: {}", e);
                None
            }
        }
    };

    // HTTP server
    let port = config.http.port;
    info!("Starting HTTP server on port {}", port);
    if let Some(ps) = &apps_state {
        shared.set_apps_state(ps.clone());
    }
    web::run_http_server_with_webrtc(
        port,
        shared.clone(),
        session_manager,
        config.http.tls,
        apps_state,
    )
    .await
    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("HTTP server error: {}", e).into()
    })?;

    Ok(())
}

fn apply_cli_overrides(config: &mut Config, args: &RunArgs) {
    config.display.width = args.width;
    config.display.height = args.height;

    if let Some(port) = args.http_port {
        config.http.port = port;
    }
    if let Some(v) = args.basic_auth_enabled {
        config.http.basic_auth_enabled = v;
    }
    if let Some(ref u) = args.basic_auth_user {
        config.http.basic_auth_user = u.clone();
    }
    if let Some(ref p) = args.basic_auth_password {
        config.http.basic_auth_password = p.clone();
    }
    if let Some(v) = args.binary_clipboard_enabled {
        config.input.enable_binary_clipboard = v;
    }
    if let Some(v) = args.commands_enabled {
        config.input.enable_commands = v;
    }
    if let Some(v) = args.terminal_enabled {
        config.ui_features.terminal_enabled = v;
        config.terminal.enabled = v;
    } else {
        config.terminal.enabled = config.terminal.enabled && config.ui_features.terminal_enabled;
    }
    if let Some(v) = args.proxy_enabled {
        config.ui_features.proxy_enabled = v;
    }
    if let Some(v) = args.console_enabled {
        config.ui_features.console_enabled = v;
    }
    if let Some(ref ft) = args.file_transfers {
        config.input.file_transfers = ft.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Some(ref d) = args.upload_dir {
        config.input.upload_dir = d.clone();
    }
    if let Some(ref c) = args.webrtc_public_candidate {
        config.webrtc.public_candidate = Some(c.clone());
    }
    if let Some(v) = args.webrtc_candidate_from_host_header {
        config.webrtc.candidate_from_host_header = v;
    }
    if args.tls {
        config.http.tls = true;
    }
}

/// Set up GTK CSS to hide headerbars on fullscreen windows.
/// Dialogs are excluded so their controls stay visible.
fn setup_gtk_css_env() {
    // Only hide headerbar on fullscreen windows, exclude dialogs
    let css = "\
window.fullscreen:not(.dialog):not(.messagedialog) headerbar {\n\
  min-height: 0;\n\
  padding: 0;\n\
  margin: 0 0 -100px 0;\n\
  border: none;\n\
  background: none;\n\
  box-shadow: none;\n\
  opacity: 0;\n\
}\n\
window.fullscreen:not(.dialog):not(.messagedialog) headerbar * {\n\
  min-height: 0;\n\
  min-width: 0;\n\
  padding: 0;\n\
  margin: 0;\n\
}\n\
window.fullscreen:not(.dialog):not(.messagedialog) .titlebar {\n\
  min-height: 0;\n\
  padding: 0;\n\
  margin: 0 0 -100px 0;\n\
  border: none;\n\
  background: none;\n\
  box-shadow: none;\n\
  opacity: 0;\n\
}\n\
window.fullscreen:not(.dialog):not(.messagedialog) .titlebar * {\n\
  min-height: 0;\n\
  min-width: 0;\n\
  padding: 0;\n\
  margin: 0;\n\
}\n\
/* Ensure dialog action areas stay visible */\n\
.dialog actionbar,\n\
.messagedialog actionbar {\n\
  min-height: 40px;\n\
}\n";

    // Write CSS to ivnc-specific directory
    let runtime_dir = env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    let ivnc_dir = format!("{}/ivnc", runtime_dir);
    if std::fs::create_dir_all(&ivnc_dir).is_err() {
        warn!("Failed to create ivnc runtime dir {}", ivnc_dir);
        return;
    }

    let css_path = format!("{}/gtk-headerbar.css", ivnc_dir);
    if std::fs::write(&css_path, css).is_err() {
        warn!("Failed to write GTK CSS to {}", css_path);
        return;
    }
    info!("Wrote GTK CSS to {}", css_path);

    // GTK_CSS tells GTK to load this CSS file in addition to the theme's CSS.
    env::set_var("GTK_CSS", &css_path);
    info!("Set GTK_CSS={}", css_path);
}

/// Returns the path to the iVnc env file: `$XDG_RUNTIME_DIR/ivnc/env`
fn env_file_path() -> String {
    let runtime_dir = env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    format!("{}/ivnc/env", runtime_dir)
}

/// Handle `ivnc env` subcommand: print export statements for the running instance.
fn handle_env_command() {
    let path = env_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            for line in content.lines() {
                if !line.is_empty() {
                    println!("export {}", line);
                }
            }
        }
        Err(_) => {
            eprintln!("iVnc is not running (env file not found: {})", path);
            std::process::exit(1);
        }
    }
}

/// Write environment variables to `$XDG_RUNTIME_DIR/ivnc/env` for external process discovery.
fn write_env_file() {
    let path = env_file_path();
    let wayland_display = env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let runtime_dir = env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    let content = format!(
        "WAYLAND_DISPLAY={}\nXDG_RUNTIME_DIR={}\nXDG_SESSION_TYPE=wayland\nGDK_BACKEND=wayland\n",
        wayland_display, runtime_dir
    );
    match std::fs::write(&path, &content) {
        Ok(_) => info!("Wrote env file to {}", path),
        Err(e) => warn!("Failed to write env file {}: {}", path, e),
    }
}

/// Remove the env file on shutdown.
fn cleanup_env_file() {
    let path = env_file_path();
    if std::fs::remove_file(&path).is_ok() {
        info!("Removed env file {}", path);
    }
}

/// Remove the Wayland socket and lock file on shutdown.
fn cleanup_wayland_socket(socket_name: &std::ffi::OsStr) {
    let runtime_dir = env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    let name = socket_name.to_string_lossy();
    let sock = format!("{}/{}", runtime_dir, name);
    let lock = format!("{}.lock", sock);
    if std::fs::remove_file(&sock).is_ok() {
        info!("Removed socket {}", sock);
    }
    if std::fs::remove_file(&lock).is_ok() {
        info!("Removed lock {}", lock);
    }
}
