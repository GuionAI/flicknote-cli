use clap::Args;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct LoginArgs {
    /// Email for OTP login
    #[arg(long, conflicts_with = "provider")]
    email: Option<String>,
    /// OAuth provider
    #[arg(long, conflicts_with = "email", value_parser = ["google", "apple"])]
    provider: Option<String>,
    /// Force re-authentication (fixes stuck sync without data loss)
    #[arg(long)]
    force: bool,
}

pub(crate) async fn run(config: &Config, args: &LoginArgs) -> Result<(), CliError> {
    let database_url = std::env::var("DATABASE_URL").ok();
    let manage_local_daemon =
        manages_daemon_after_login_for(cfg!(target_os = "macos"), database_url.as_deref());
    if config.paths.session_file.exists() {
        if !args.force {
            return Err(CliError::Other(
                "Already logged in. Use `flicknote login --force` to re-authenticate (e.g. after sync issues).".into(),
            ));
        }
        // --force: stop daemon and clear stale session before re-auth
        if manage_local_daemon {
            super::daemon::stop(config)?;
            super::daemon::uninstall()?;
        }
        std::fs::remove_file(&config.paths.session_file)?;
    }

    let client = GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    );

    if let Some(ref provider) = args.provider {
        client
            .sign_in_with_oauth(provider)
            .await
            .map_err(|e| CliError::Auth {
                operation: "signIn".into(),
                description: e.to_string(),
            })?;
    } else {
        let email = match &args.email {
            Some(e) => e.clone(),
            None => {
                eprint!("Email: ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                input.trim().to_string()
            }
        };

        client
            .sign_in_with_otp(&email)
            .await
            .map_err(|e| CliError::Auth {
                operation: "signIn".into(),
                description: e.to_string(),
            })?;
        println!("OTP sent to {email}");

        eprint!("Enter code: ");
        let mut code = String::new();
        std::io::stdin().read_line(&mut code)?;

        client
            .verify_otp(&email, code.trim())
            .await
            .map_err(|e| CliError::Auth {
                operation: "verifyOtp".into(),
                description: e.to_string(),
            })?;
    }

    println!("Authenticated");

    if manage_local_daemon {
        // The macOS login flow owns the per-user LaunchAgent lifecycle.
        super::sync::install_local_daemon(config, std::time::Duration::from_secs(10)).await?;
        println!("Sync daemon installed and started");
    }

    Ok(())
}

const fn manages_daemon_after_login_for(target_is_macos: bool, database_url: Option<&str>) -> bool {
    target_is_macos && database_url.is_none()
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_login_does_not_wait_for_a_launchd_daemon() {
        assert!(!super::manages_daemon_after_login_for(false, None));
    }

    #[test]
    fn managed_login_never_manages_the_local_launch_agent() {
        assert!(!super::manages_daemon_after_login_for(
            true,
            Some("postgres://managed")
        ));
        assert!(super::manages_daemon_after_login_for(true, None));
        assert!(!super::manages_daemon_after_login_for(false, None));
    }
}
