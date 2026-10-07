//! Owned HTTP/PowerSync and rendered presentation evidence, never cloud state.
use super::*;
use gpui_kit::test::TestWindowExt;
use serde_json::json;

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // A single SDK stream carries staged checkpoint readiness.
fn sdk_first_sync_waits_for_every_default_checkpoint(cx: &mut gpui_kit::TestAppContext) {
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
    let (requests, mut receive) = tokio::sync::mpsc::channel(8);
    let server = runtime.spawn(async move {
        let router = axum::Router::new().route(
            "/sync/stream",
            axum::routing::post(move |axum::Json(request): axum::Json<serde_json::Value>| {
                let requests = requests.clone();
                async move {
                    let (send, lines) = tokio::sync::mpsc::channel::<String>(8);
                    requests
                        .send((
                            send,
                            request["streams"]["subscriptions"]
                                .as_array()
                                .is_some_and(|s| {
                                    s.iter().any(|s| s["stream"] == "conversation_detail")
                                }),
                        ))
                        .await
                        .unwrap();
                    let body = futures_lite::stream::unfold(lines, |mut lines| async move {
                        lines
                            .recv()
                            .await
                            .map(|line| (Ok::<_, std::io::Error>(line), lines))
                    });
                    (
                        [("content-type", "application/json")],
                        axum::body::Body::from_stream(body),
                    )
                }
            }),
        );
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = flicknote_core::profile::load(root.path()).unwrap();
    config.supabase_url.clone_from(&origin);
    config.supabase_anon_key = "owned-key".into();
    config.powersync_url.clone_from(&origin);
    config.api_url.clone_from(&origin);
    config.gateway_url = origin;
    flicknote_auth::session::save_session(
        &config.paths.session_file,
        &flicknote_auth::client::AuthSession {
            access_token: "owned-access".into(),
            refresh_token: "owned-refresh".into(),
            expires_at: Some(u64::MAX),
            user: flicknote_auth::client::AuthUser {
                id: "owned-user".into(),
                email: None,
            },
        },
    )
    .unwrap();
    let (_cancel, cancel) = tokio::sync::watch::channel(false);
    let host = runtime
        .block_on(flicknote_sync::LocalHost::start(config, Some(0), cancel))
        .unwrap();
    let optional = host.db.sync_stream("conversation_detail", None);
    let _subscription = runtime.block_on(optional.subscribe()).unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        user_id: host.user_id.clone(),
        real_account: true,
        first_sync: Mutex::default(),
        destination: Mutex::default(),
        temporal: Mutex::default(),
        capture: Arc::default(),
        draft: Mutex::default(),
        organization: Mutex::default(),
        source: crate::source::Control::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
    });
    cx.update(gpui_kit::init);
    let (window, view) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    let send = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (send, subscribed) = receive.recv().await.unwrap();
                if subscribed {
                    break send;
                }
            }
        })
        .await
        .unwrap()
    });
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).sync_progress == Some(crate::sync_progress::Progress::Indeterminate)
        })
    });
    assert_bar(
        cx,
        window,
        &view,
        Some(crate::sync_progress::Progress::Indeterminate),
    );
    let emit =
        |value: serde_json::Value| runtime.block_on(send.send(format!("{value}\n"))).unwrap();
    emit(json!({"checkpoint": {
        "last_op_id":"2", "buckets":[
            {"bucket":"notes-owned","priority":1,"checksum":0,"count":2,"subscriptions":[{"default":0}]},
            {"bucket":"metadata-owned","priority":4,"checksum":0,"count":0,"subscriptions":[{"default":1}]},
            {"bucket":"optional-owned","priority":8,"checksum":0,"count":0,"subscriptions":[{"sub":0}]}
        ], "streams":[
            {"name":"notes","is_default":true,"errors":[]},
            {"name":"user_data","is_default":true,"errors":[]},
            {"name":"conversation_detail","is_default":false,"errors":[]}
        ]
    }}));
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).sync_message.as_deref() == Some("First sync: 0% — Downloading notes…")
        })
    });
    assert_bar(
        cx,
        window,
        &view,
        Some(crate::sync_progress::Progress::Percent(0)),
    );
    for (op, percent) in [(1, 45), (2, 90)] {
        emit(json!({"data":{"bucket":"notes-owned","data":[{
            "op_id":op.to_string(), "op":"PUT", "object_id":format!("note-{op}"),
            "object_type":"notes", "checksum":0,
            "data":json!({"user_id":"owned-user","short_id":op,"content":"Owned fixture","type":"normal","status":"ready","created_at":"2020-01-01T12:00:00Z"}).to_string()
        }]}}));
        let expected = if percent == 90 {
            "First sync: 90% — Applying notes…"
        } else {
            "First sync: 45% — Downloading notes…"
        };
        tests::settle(cx, |cx| {
            cx.update(|cx| view.read(cx).sync_message.as_deref() == Some(expected))
        });
        assert_bar(
            cx,
            window,
            &view,
            Some(crate::sync_progress::Progress::Percent(percent)),
        );
        assert!(!services.first_sync.lock().unwrap().complete);
    }
    emit(json!({"partial_checkpoint_complete":{"last_op_id":"2","priority":1}}));
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).sync_message.as_deref() == Some("First sync: 90% — Finishing sync…")
        })
    });
    assert_bar(
        cx,
        window,
        &view,
        Some(crate::sync_progress::Progress::Percent(90)),
    );
    assert!(!services.first_sync.lock().unwrap().complete);
    emit(json!({"partial_checkpoint_complete":{"last_op_id":"2","priority":4}}));
    tests::settle(cx, |cx| cx.update(|cx| view.read(cx).first_synced));
    assert_bar(cx, window, &view, None);
    // Approved quiet ongoing presentation: stable list geometry through normal activity.
    let viewport = cx
        .update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("today-notes").bounds(),
                window.find("list-status").bounds(),
            )
        })
        .unwrap();
    for (connecting, downloading) in [(false, false), (true, false), (false, true), (false, false)]
    {
        let status = crate::sync_progress::Snapshot {
            connected: !connecting,
            connecting,
            error: false,
            required_ready: None,
            notes_applied: false,
            notes_progress: downloading.then_some((100, 50)),
        };
        let mut first = services.first_sync.lock().unwrap();
        let message = first.update(status);
        let progress = first.progress;
        drop(first);
        assert_eq!(message, None);
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |this, _| {
                this.sync_message = message;
                this.sync_progress = progress;
            });
            window.render_frame(cx);
            assert!(window.try_find("first-sync-progress").is_none());
            assert!(window.try_find("sync-message").is_none());
            assert_eq!(
                (
                    window.find("today-notes").bounds(),
                    window.find("list-status").bounds()
                ),
                viewport
            );
        })
        .unwrap();
    }
    // An active optional subscription has not applied; default readiness is enough.
    let status = host.db.status();
    let optional_status = status.for_stream(&optional).unwrap();
    assert!(optional_status.subscription.is_active());
    assert!(!optional_status.subscription.is_default());
    assert!(!optional_status.subscription.has_synced());
    emit(json!({"checkpoint_complete":{"last_op_id":"2"}}));
    tests::settle(cx, |cx| {
        cx.update(|cx| view.read(cx).sync_message.is_none())
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).composer.read(cx).value().is_empty());
        window.remove_window();
    })
    .unwrap();
    // Reopening shares the completed state; no first-sync flicker is introduced.
    let reopened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap()
        .1
    });
    tests::settle(cx, |cx| cx.update(|cx| reopened.read(cx).first_synced));
    cx.update(|cx| assert_eq!(reopened.read(cx).sync_message, None));
    services.cancel_operations();
    runtime.block_on(host.shutdown());
    server.abort();
}

fn assert_bar(
    cx: &mut gpui_kit::TestAppContext,
    window: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    view: &Entity<Today>,
    expected: Option<crate::sync_progress::Progress>,
) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).sync_progress, expected);
        let bar = window.try_find("first-sync-progress");
        assert_eq!(bar.is_some(), expected.is_some());
        if let Some(bar) = bar {
            assert!(bar.visible());
            assert!(bar.bounds().size.width > px(16.));
            assert!(bar.bounds().size.height > px(8.));
        }
    })
    .unwrap();
}
