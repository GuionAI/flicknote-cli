use super::daemon_lifecycle::{DaemonLifecycle, NativeDaemonLifecycle};
use async_trait::async_trait;
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
    /// Force re-authentication after removing the current daemon service and session
    #[arg(long)]
    force: bool,
}

struct GoTrueAuthenticator;

#[async_trait]
trait LoginAuthenticator: Send + Sync {
    async fn authenticate(&self, config: &Config, args: &LoginArgs) -> Result<(), CliError>;
}

#[async_trait]
impl LoginAuthenticator for GoTrueAuthenticator {
    async fn authenticate(&self, config: &Config, args: &LoginArgs) -> Result<(), CliError> {
        let client = GoTrueClient::new(
            &config.supabase_url,
            &config.supabase_anon_key,
            &config.paths.session_file,
        );
        if let Some(provider) = &args.provider {
            return client
                .sign_in_with_oauth(provider)
                .await
                .map(|_| ())
                .map_err(|error| CliError::Auth {
                    operation: "signIn".into(),
                    description: error.to_string(),
                });
        }

        let email = match &args.email {
            Some(email) => email.clone(),
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
            .map_err(|error| CliError::Auth {
                operation: "signIn".into(),
                description: error.to_string(),
            })?;
        println!("OTP sent to {email}");

        eprint!("Enter code: ");
        let mut code = String::new();
        std::io::stdin().read_line(&mut code)?;
        client
            .verify_otp(&email, code.trim())
            .await
            .map(|_| ())
            .map_err(|error| CliError::Auth {
                operation: "verifyOtp".into(),
                description: error.to_string(),
            })
    }
}

pub(crate) async fn run(config: &Config, args: &LoginArgs) -> Result<(), CliError> {
    run_with_dependencies(config, args, &NativeDaemonLifecycle, &GoTrueAuthenticator).await
}

async fn run_with_dependencies(
    config: &Config,
    args: &LoginArgs,
    lifecycle: &dyn DaemonLifecycle,
    authenticator: &dyn LoginAuthenticator,
) -> Result<(), CliError> {
    if config.paths.session_file.exists() && !args.force {
        return Err(CliError::Other(
            "Already logged in. Use `flicknote login --force` to re-authenticate.".into(),
        ));
    }

    if args.force {
        lifecycle.uninstall(config).await?;
        if config.paths.session_file.exists() {
            std::fs::remove_file(&config.paths.session_file)?;
        }
    }

    authenticator.authenticate(config, args).await?;
    println!("Authenticated");
    if let Err(error) = lifecycle.install_and_wait(config).await {
        println!("Daemon startup failed: {error}");
        return Err(CliError::Other(
            "Authentication succeeded but the daemon is not ready; run `flicknote daemon status --verbose` and retry `flicknote daemon install`".to_string(),
        ));
    }
    println!("FlickNote daemon ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::daemon_lifecycle::ServiceCleanup;
    use std::sync::{Arc, Mutex};

    struct FakeLifecycle {
        events: Arc<Mutex<Vec<&'static str>>>,
        cleanup_error: bool,
        install_error: bool,
    }

    #[async_trait]
    impl DaemonLifecycle for FakeLifecycle {
        async fn install_and_wait(&self, _config: &Config) -> Result<(), CliError> {
            self.events.lock().unwrap().push("install");
            if self.install_error {
                return Err(CliError::Other("readiness failed".to_string()));
            }
            Ok(())
        }

        async fn uninstall(&self, _config: &Config) -> Result<ServiceCleanup, CliError> {
            self.events.lock().unwrap().push("uninstall");
            if self.cleanup_error {
                return Err(CliError::Other("cleanup failed".to_string()));
            }
            Ok(ServiceCleanup::Removed)
        }
    }

    struct FakeAuthenticator {
        events: Arc<Mutex<Vec<&'static str>>>,
        succeeds: bool,
    }

    #[async_trait]
    impl LoginAuthenticator for FakeAuthenticator {
        async fn authenticate(&self, config: &Config, _args: &LoginArgs) -> Result<(), CliError> {
            self.events.lock().unwrap().push("authenticate");
            if !self.succeeds {
                return Err(CliError::Other("authentication failed".to_string()));
            }
            std::fs::write(&config.paths.session_file, "new session")?;
            Ok(())
        }
    }

    fn config(directory: &std::path::Path) -> Config {
        Config {
            supabase_url: "http://127.0.0.1:9".to_string(),
            supabase_anon_key: "key".to_string(),
            powersync_url: "http://127.0.0.1:9".to_string(),
            api_url: "http://127.0.0.1:9".to_string(),
            gateway_url: "http://127.0.0.1:9".to_string(),
            web_url: None,
            paths: flicknote_core::config::ConfigPaths {
                config_dir: directory.to_path_buf(),
                data_dir: directory.to_path_buf(),
                config_file: directory.join("config.json"),
                session_file: directory.join("session.json"),
                db_file: directory.join("db"),
                log_file: directory.join("log"),
            },
        }
    }

    fn force_args() -> LoginArgs {
        LoginArgs {
            email: Some("person@example.com".to_string()),
            provider: None,
            force: true,
        }
    }

    fn dependencies(
        events: &Arc<Mutex<Vec<&'static str>>>,
        cleanup_error: bool,
        install_error: bool,
        authentication_succeeds: bool,
    ) -> (FakeLifecycle, FakeAuthenticator) {
        (
            FakeLifecycle {
                events: Arc::clone(events),
                cleanup_error,
                install_error,
            },
            FakeAuthenticator {
                events: Arc::clone(events),
                succeeds: authentication_succeeds,
            },
        )
    }

    #[tokio::test]
    async fn forced_login_cleans_stale_service_without_a_session_before_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let events = Arc::new(Mutex::new(Vec::new()));
        let (lifecycle, authenticator) = dependencies(&events, false, false, false);

        assert!(
            run_with_dependencies(&config, &force_args(), &lifecycle, &authenticator)
                .await
                .is_err()
        );

        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["uninstall", "authenticate"]
        );
        assert!(!config.paths.session_file.exists());
    }

    #[tokio::test]
    async fn forced_login_cleanup_failure_preserves_old_session_and_skips_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        std::fs::write(&config.paths.session_file, "old session").unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let (lifecycle, authenticator) = dependencies(&events, true, false, true);

        assert!(
            run_with_dependencies(&config, &force_args(), &lifecycle, &authenticator)
                .await
                .is_err()
        );

        assert_eq!(events.lock().unwrap().as_slice(), ["uninstall"]);
        assert_eq!(
            std::fs::read_to_string(&config.paths.session_file).unwrap(),
            "old session"
        );
    }

    #[tokio::test]
    async fn failed_forced_authentication_does_not_restore_the_old_session() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        std::fs::write(&config.paths.session_file, "old session").unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let (lifecycle, authenticator) = dependencies(&events, false, false, false);

        assert!(
            run_with_dependencies(&config, &force_args(), &lifecycle, &authenticator)
                .await
                .is_err()
        );

        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["uninstall", "authenticate"]
        );
        assert!(!config.paths.session_file.exists());
    }

    #[tokio::test]
    async fn daemon_install_failure_retains_the_new_authenticated_session() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let events = Arc::new(Mutex::new(Vec::new()));
        let (lifecycle, authenticator) = dependencies(&events, false, true, true);

        assert!(
            run_with_dependencies(&config, &force_args(), &lifecycle, &authenticator)
                .await
                .is_err()
        );

        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["uninstall", "authenticate", "install"]
        );
        assert_eq!(
            std::fs::read_to_string(&config.paths.session_file).unwrap(),
            "new session"
        );
    }
}
