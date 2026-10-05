//! Local fake emails only; rendered input is not native OS evidence.
use super::*;
use axum::{Json, Router, response::IntoResponse, routing::post};
use clap::Parser;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, WindowOptions};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

fn settle(cx: &mut TestAppContext, predicate: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if predicate(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Login operation deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn options(root: &std::path::Path) -> crate::launch::Options {
    crate::launch::Options::try_parse_from([
        "trial",
        "--profile",
        root.to_str().unwrap(),
        "--mcp-port",
        "0",
    ])
    .unwrap()
}
fn start(
    runtime: &tokio::runtime::Runtime,
    root: &std::path::Path,
) -> (
    watch::Sender<bool>,
    oneshot::Receiver<LoginHandle>,
    oneshot::Receiver<Result<crate::launch::Host, String>>,
) {
    let (quit, receiver) = watch::channel(false);
    let (login, pane) = oneshot::channel();
    let (host, result) = oneshot::channel();
    let options = options(root);
    runtime.spawn(async move {
        let result = options
            .start(receiver, move |handle| {
                assert!(login.send(handle).is_ok());
            })
            .await;
        assert!(host.send(result).is_ok());
    });
    (quit, pane, result)
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned HTTP/profile/window verifies the complete login and cancellation sequence.
fn email_code_profile_host_and_cancelled_window(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let sends = Arc::new(AtomicUsize::new(0));
    let verifies = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let verify_gate = Arc::new(Notify::new());
    let completed = Arc::new(Notify::new());
    let router = Router::new().route("/auth/v1/otp", post({
        let sends = sends.clone(); let gate = gate.clone();
        move |Json(body): Json<Value>| { let sends = sends.clone(); let gate = gate.clone(); async move {
            sends.fetch_add(1, Ordering::SeqCst);
            if body["email"] == "unavailable@example.test" { return axum::http::StatusCode::SERVICE_UNAVAILABLE; }
            gate.notified().await;
            axum::http::StatusCode::OK
        }}
    })).route("/auth/v1/verify", post({
        let verifies = verifies.clone(); let gate = verify_gate.clone(); let completed = completed.clone();
        move |Json(body): Json<Value>| { let verifies = verifies.clone(); let gate = gate.clone(); let completed = completed.clone(); async move {
            verifies.fetch_add(1, Ordering::SeqCst);
            if body["token"] == "unavailable" { return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(); }
            if body["token"] == "delayed" { gate.notified().await; }
            if body["token"] != "correct" && body["token"] != "delayed" {
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"message":"invalid or expired"}))).into_response();
            }
            assert_eq!(body["email"], "owned@example.test");
            assert!(body["code_verifier"].as_str().is_some_and(|s| !s.is_empty()));
            completed.notify_one();
            Json(json!({"access_token":"owned-access","refresh_token":"owned-refresh","expires_at":u64::MAX,"user":{"id":"otp-test-account"}})).into_response()
        }}
    })).fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
    let server = runtime.spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let config = flicknote_core::profile::load(root.path()).unwrap();
    std::fs::write(&config.paths.config_file, serde_json::to_vec(&json!({"supabaseUrl":origin,"supabaseAnonKey":"owned-key","powersyncUrl":origin,"apiUrl":origin,"gatewayUrl":origin})).unwrap()).unwrap();
    let (quit, pane, host) = start(&runtime, root.path());
    let handle = runtime.block_on(pane).unwrap();
    // Ownership precedes all GoTrue effects, including the PKCE/session write.
    let (_other_quit, other) = watch::channel(false);
    let conflict = runtime
        .block_on(options(root.path()).start(other, |_| panic!("competing owner showed login")));
    assert!(conflict.is_err());
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    assert!(!config.paths.session_file.exists());
    cx.update(gpui_kit::init);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| LoginPane::new(handle.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.email.update(cx, |input, cx| {
                input.set_value("unavailable@example.test", window, cx)
            });
            this.send(false, window, cx);
        })
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).error.is_some()));
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).email.read(cx).value(),
            "unavailable@example.test"
        );
        assert_eq!(
            view.read(cx).error.as_deref(),
            Some("Could not complete sign-in. Try again.")
        )
    });
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.email.update(cx, |input, cx| {
                input.set_value("owned@example.test", window, cx)
            });
            this.send(false, window, cx);
            this.send(false, window, cx); // Busy prevents a duplicate even before rendering disabled state.
        })
    })
    .unwrap();
    settle(cx, |_| sends.load(Ordering::SeqCst) == 2);
    cx.update(|cx| assert!(view.read(cx).busy));
    gate.notify_one();
    settle(cx, |cx| cx.update(|cx| view.read(cx).code_sent));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-email", cx);
        assert!(!view.read(cx).code_sent);
        assert_eq!(view.read(cx).email.read(cx).value(), "owned@example.test");
        window.render_frame(cx);
        window.click("send-code", cx);
    })
    .unwrap();
    gate.notify_one();
    settle(cx, |cx| cx.update(|cx| view.read(cx).code_sent));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("resend-code", cx);
    })
    .unwrap();
    gate.notify_one();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).busy));
    assert_eq!(sends.load(Ordering::SeqCst), 4);
    for code in ["wrong", "expired", "unavailable"] {
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |this, cx| {
                this.code
                    .update(cx, |input, cx| input.set_value(code, window, cx));
                this.send(true, window, cx);
            })
        })
        .unwrap();
        settle(cx, |cx| cx.update(|cx| view.read(cx).error.is_some()));
        cx.update(|cx| {
            assert_eq!(view.read(cx).email.read(cx).value(), "owned@example.test");
            assert!(view.read(cx).code_sent);
            assert_eq!(
                view.read(cx).error.as_deref(),
                Some("Could not complete sign-in. Try again.")
            );
        });
        assert!(!config.paths.session_file.exists());
    }
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.code
                .update(cx, |input, cx| input.set_value("correct", window, cx));
            this.send(true, window, cx);
        })
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).busy));
    let host = runtime.block_on(host).unwrap().unwrap();
    let crate::launch::Host::Real(host) = host else {
        panic!("fixture fallback");
    };
    assert_eq!(host.user_id, "otp-test-account");
    assert_eq!(
        flicknote_auth::session::load_session(&config.paths.session_file)
            .unwrap()
            .access_token,
        "owned-access"
    );
    assert!(
        runtime
            .block_on(flicknote_client::DaemonClient::new(&host.socket).health())
            .is_ok()
    );
    let _entered = runtime.enter();
    let mut today =
        flicknote_sync::today::TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    runtime.block_on(async {
        while today.receiver.borrow().is_none() {
            today.receiver.changed().await.unwrap();
        }
    });
    assert!(
        today
            .receiver
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows
            .is_empty()
    );
    drop(today);
    runtime.block_on(host.shutdown());
    let (_cancel, receiver) = watch::channel(false);
    let reopened = runtime
        .block_on(
            options(root.path()).start(receiver, |_| panic!("usable session did not skip login")),
        )
        .unwrap();
    let crate::launch::Host::Real(reopened) = reopened else {
        panic!("fixture fallback");
    };
    runtime.block_on(reopened.shutdown());
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drop(quit);
    std::fs::remove_file(&config.paths.session_file).unwrap();
    let (quit, pane, mut host) = start(&runtime, root.path());
    let handle = runtime.block_on(pane).unwrap();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| LoginPane::new(handle.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.email.update(cx, |input, cx| {
                input.set_value("owned@example.test", window, cx)
            });
            this.send(false, window, cx);
        })
    })
    .unwrap();
    gate.notify_one();
    settle(cx, |cx| cx.update(|cx| view.read(cx).code_sent));
    // Clear the prior successful fake server notification before delayed verify.
    runtime.block_on(completed.notified());
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.code
                .update(cx, |input, cx| input.set_value("delayed", window, cx));
            this.send(true, window, cx);
        })
    })
    .unwrap();
    settle(cx, |_| verifies.load(Ordering::SeqCst) == 5);
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    cx.run_until_parked();
    verify_gate.notify_one(); // An old verification response cannot open a host after window close.
    let (window, fresh) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| LoginPane::new(handle.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        fresh.update(cx, |this, cx| {
            this.email.update(cx, |input, cx| {
                input.set_value("owned@example.test", window, cx)
            });
            this.send(false, window, cx);
        })
    })
    .unwrap();
    // Processing this request while the old verify was delayed proves its window cancellation.
    settle(cx, |_| sends.load(Ordering::SeqCst) == 6);
    assert!(matches!(
        host.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    quit.send_replace(true);
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(3), &mut host)
                    .await
                    .unwrap()
                    .unwrap()
            })
            .is_err()
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(fresh);
    gate.notify_one();
    assert!(!config.paths.session_file.exists());
    assert!(!config.paths.data_dir.join("daemon.sock").exists());
    assert!(flicknote_sync::ownership::DataDirectoryLock::acquire(&config.paths.data_dir).is_ok());
    assert_eq!(sends.load(Ordering::SeqCst), 6);
    server.abort();
}

/// Inert local request channel: layout checks cannot send an email or open a host.
#[gpui_kit::test]
fn login_workbench_themes_and_error_actions_are_reachable(cx: &mut TestAppContext) {
    use gpui_kit::component::ThemeMode;
    use gpui_kit::{Bounds, WindowBounds, point, px, size};
    cx.update(gpui_kit::init);
    let (sender, _requests) = mpsc::channel(1);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(760.), px(560.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| LoginPane::new(LoginHandle(sender), window, cx)),
        )
        .unwrap()
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update_window(window, |_, window, cx| {
            crate::workspace::apply_theme(mode, cx);
            for code_sent in [false, true] {
                view.update(cx, |this, cx| {
                    this.code_sent = code_sent;
                    this.error = Some("Synthetic sign-in failure. Try again.".into());
                    this.email.update(cx, |input, cx| {
                        input.set_value("owned@example.test", window, cx)
                    });
                    cx.notify();
                });
                window.render_frame(cx);
                let form = window.find("login-form").bounds();
                let outer = window.find("login-workspace").bounds();
                assert_eq!(form.size.width, px(360.));
                assert!(form.origin.x >= outer.origin.x && form.right() <= outer.right());
                assert!(form.origin.y >= outer.origin.y && form.bottom() <= outer.bottom());
                let actions: &[&str] = if code_sent {
                    &["verify-code", "resend-code", "change-email"]
                } else {
                    &["send-code"]
                };
                for id in actions {
                    let button = window.find(*id).bounds();
                    assert!(button.origin.x >= form.origin.x && button.right() <= form.right());
                    assert!(button.origin.y >= form.origin.y && button.bottom() <= form.bottom());
                }
                assert!(window.find("login-error").bounds().size.height > px(0.));
                assert_eq!(Theme::global(cx).is_dark(), mode.is_dark());
                assert_eq!(
                    Theme::global(cx).semantic_tokens(),
                    gpui_kit::base::Theme::global(cx).tokens
                );
            }
        })
        .unwrap();
    }
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
}
