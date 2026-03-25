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

pub(crate) fn run(config: &Config, args: &LoginArgs) -> Result<(), CliError> {
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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let client = GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    );

    rt.block_on(async {
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
        Ok::<(), CliError>(())
    })?;

    // Install launchd service — this boots out any existing service first,
    // then bootstraps fresh. The daemon starts immediately (KeepAlive + RunAtLoad)
    // and creates the local DB on startup.
    super::daemon::install(config)?;
    println!("Sync daemon installed and started");

    Ok(())
}
