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
    if config.paths.session_file.exists() {
        if !args.force {
            return Err(CliError::Other(
                "Already logged in. Use `flicknote login --force` to re-authenticate (e.g. after sync issues).".into(),
            ));
        }
        // --force: stop daemon and clear stale session before re-auth
        super::daemon::stop(config)?;
        super::daemon::uninstall()?;
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

    if manages_daemon_after_login() {
        // The macOS login flow owns the per-user LaunchAgent lifecycle.
        super::daemon::install(config)?;
        super::sync::wait_for_daemon_ready(
            config,
            std::time::Duration::from_secs(10),
            std::time::Duration::from_millis(100),
        )
        .await?;
        println!("Sync daemon installed and started");
    }

    Ok(())
}

const fn manages_daemon_after_login() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_login_does_not_wait_for_a_launchd_daemon() {
        assert!(!super::manages_daemon_after_login());
    }
}
