use clap::Args;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

#[derive(Args)]
pub struct LoginArgs {
    /// Email for OTP login
    #[arg(long, conflicts_with = "provider")]
    email: Option<String>,
    /// OAuth provider
    #[arg(long, conflicts_with = "email", value_parser = ["google", "apple"])]
    provider: Option<String>,
}

pub fn run(config: &Config, args: &LoginArgs) -> Result<(), CliError> {
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
        Ok(())
    })
}
