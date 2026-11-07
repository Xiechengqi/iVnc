use clap::{Parser, Subcommand};
use std::{net::IpAddr, path::PathBuf};

use crate::config;

#[derive(Parser, Debug)]
#[command(name = "ivnc")]
#[command(author = "iVnc Team")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "iVnc streaming core", long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub run_args: RunArgs,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the compositor (default when no subcommand given)
    Run(RunArgs),
    /// Print environment variables for connecting to a running iVnc instance
    Env,
}

#[derive(Parser, Debug, Clone)]
pub struct RunArgs {
    #[arg(long, hide = true)]
    pub app_supervisor: Option<PathBuf>,
    #[arg(long, hide = true)]
    pub app_worker: Option<PathBuf>,
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/ivnc.toml")]
    pub config: PathBuf,

    /// Display width
    #[arg(long, default_value = "1920")]
    pub width: u32,

    /// Display height
    #[arg(long, default_value = "1080")]
    pub height: u32,

    /// HTTP port for health/UI
    #[arg(long)]
    pub http_port: Option<u16>,

    /// Enable or disable basic authentication (true/false)
    #[arg(long)]
    pub basic_auth_enabled: Option<bool>,

    /// Basic authentication username
    #[arg(long)]
    pub basic_auth_user: Option<String>,

    /// Basic authentication password
    #[arg(long)]
    pub basic_auth_password: Option<String>,

    /// Enable or disable binary clipboard (true/false)
    #[arg(long)]
    pub binary_clipboard_enabled: Option<bool>,

    /// Enable or disable client command execution (true/false)
    #[arg(long)]
    pub commands_enabled: Option<bool>,

    /// Enable or disable the browser toolbar terminal (true/false)
    #[arg(long)]
    pub terminal_enabled: Option<bool>,

    /// Enable or disable the built-in Miao app and toolbar proxy panel (true/false)
    #[arg(long)]
    pub proxy_enabled: Option<bool>,

    /// Deploy or recover this VPS with Miao and start global proxy mode
    #[arg(long, value_name = "IP")]
    pub proxy_ip: Option<IpAddr>,

    /// Override Miao autostart for this iVnc run (true/false)
    #[arg(long, value_name = "BOOL")]
    pub proxy_autostart: Option<bool>,

    /// Enable or disable remote console access (true/false)
    #[arg(long)]
    pub console_enabled: Option<bool>,

    /// Allowed file transfer directions (comma-separated: "upload,download")
    #[arg(long)]
    pub file_transfers: Option<String>,

    /// Directory to store uploaded files
    #[arg(long)]
    pub upload_dir: Option<String>,

    /// Public candidate address for ICE-TCP (e.g., "1.2.3.4:8008")
    #[arg(long)]
    pub webrtc_public_candidate: Option<String>,

    /// Allow candidate override from Host header (true/false)
    #[arg(long)]
    pub webrtc_candidate_from_host_header: Option<bool>,

    /// Verbose logging
    #[arg(short, long, action)]
    pub verbose: bool,

    /// Run in foreground (don't daemonize)
    #[arg(long, action)]
    pub foreground: bool,

    /// Enable HTTPS with auto-generated self-signed certificate
    #[arg(long, action)]
    pub tls: bool,

    /// PID file path (empty = $IVNC_HOME/ivnc.pid)
    #[arg(long, default_value = "")]
    pub pidfile: String,
}

impl RunArgs {
    pub fn load_config(&self) -> Result<config::Config, Box<dyn std::error::Error>> {
        config::Config::load(&self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_autostart_is_an_optional_boolean_in_both_run_forms() {
        assert!(Cli::try_parse_from(["ivnc"])
            .unwrap()
            .run_args
            .proxy_autostart
            .is_none());
        for value in ["true", "false"] {
            for args in [
                vec!["ivnc", "--proxy-autostart", value],
                vec!["ivnc", "run", "--proxy-autostart", value],
            ] {
                let cli = Cli::try_parse_from(args).unwrap();
                let run = match cli.command {
                    Some(Command::Run(run)) => run,
                    _ => cli.run_args,
                };
                assert_eq!(run.proxy_autostart, Some(value == "true"));
            }
        }
        for args in [
            vec!["ivnc", "--proxy-autostart"],
            vec!["ivnc", "--proxy-autostart", "invalid"],
            vec![
                "ivnc",
                "--proxy-autostart",
                "true",
                "--proxy-autostart",
                "false",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn proxy_ip_is_optional_and_accepts_ipv4_and_ipv6_in_both_run_forms() {
        assert!(Cli::try_parse_from(["ivnc"])
            .unwrap()
            .run_args
            .proxy_ip
            .is_none());
        for address in ["203.0.113.10", "2001:db8::10"] {
            for args in [
                vec!["ivnc", "--proxy-ip", address],
                vec!["ivnc", "run", "--proxy-ip", address],
            ] {
                let cli = Cli::try_parse_from(args).unwrap();
                let run = match cli.command {
                    Some(Command::Run(run)) => run,
                    _ => cli.run_args,
                };
                assert_eq!(run.proxy_ip, Some(address.parse().unwrap()));
            }
        }
        let cli = Cli::try_parse_from(["ivnc", "--proxy-ip=203.0.113.10"]).unwrap();
        assert_eq!(cli.run_args.proxy_ip, Some("203.0.113.10".parse().unwrap()));
    }

    #[test]
    fn proxy_ip_rejects_missing_invalid_or_duplicate_addresses() {
        for args in [
            vec!["ivnc", "--proxy-ip"],
            vec!["ivnc", "--proxy-ip", "999.1.2.3"],
            vec!["ivnc", "--proxy-ip", "vps.example.com"],
            vec!["ivnc", "--proxy-ip", "203.0.113.10;echo bad"],
            vec!["ivnc", "--proxy-ip", "203.0.113.10:22"],
            vec![
                "ivnc",
                "--proxy-ip",
                "203.0.113.10",
                "--proxy-ip",
                "203.0.113.11",
            ],
        ] {
            assert!(Cli::try_parse_from(args.clone()).is_err(), "{args:?}");
        }
    }
}
