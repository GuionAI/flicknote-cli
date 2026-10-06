//! Owned LocalHost/GPUI windows only: no native application or live state.
use super::tests::settle;
use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt};

fn fixture(runtime: &tokio::runtime::Runtime, root: &std::path::Path) -> flicknote_sync::LocalHost {
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router =
        axum::Router::new().fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
    runtime.spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = flicknote_core::profile::load(root).unwrap();
    config.supabase_url.clone_from(&origin);
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
                id: "append-owner".into(),
                email: None,
            },
        },
    )
    .unwrap();
    let host = runtime
        .block_on(flicknote_sync::LocalHost::start(
            config,
            Some(0),
            tokio::sync::watch::channel(false).1,
        ))
        .unwrap();
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        for (id, content) in [(1, ""), (2, "# Original\n\nBody")] {
            writer.execute("INSERT INTO notes(id,short_id,user_id,content,type,status,is_flagged,title,metadata,created_at) VALUES(?,?,'append-owner',?,'normal','ready',0,'Original title','{\"created_by_ai\":true}',strftime('%Y-%m-%dT%H:%M:%SZ','now'))", [format!("00000000-0000-4000-8000-{id:012}"),id.to_string(),content.to_owned()]).unwrap();
        }
    });
    host
}
fn services(host: &flicknote_sync::LocalHost, runtime: &tokio::runtime::Runtime) -> Arc<Services> {
    Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        user_id: host.user_id.clone(),
        real_account: false,
        first_sync: Mutex::default(),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
    })
}
fn open(
    cx: &mut TestAppContext,
    services: Arc<Services>,
) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, Entity<Today>) {
    cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services, window, cx))
        })
        .unwrap();
        (window.downcast().unwrap(), view)
    })
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned host checks immutable identity through close/reopen.
fn append_local_host_optimism_busy_watch_copy_and_close_reopen(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = fixture(&runtime, root.path());
    let services = services(&host, &runtime);
    cx.update(gpui_kit::init);
    let (window, view) = open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    // The open append context retains the same native composition Return guard.
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.select(2, window, cx);
            this.composer.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
            });
        });
        window.render_frame(cx);
        window.press("enter", cx);
        assert!(view.read(cx).model.capture().appends.pending.is_empty());
        view.read(cx).composer.clone().update(cx, |input, cx| {
            input.replace_text_in_range(None, "", window, cx)
        });
    })
    .unwrap();
    // Hold the existing database writer seam; accepted work must outlive its window.
    let writer = runtime.block_on(host.db.writer()).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.select(2, window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("  **added**\n", window, cx));
            this.submit(window, cx);
            assert!(this.composer.read(cx).value().is_empty());
            assert_eq!(
                this.detail.as_ref().unwrap().source,
                "# Original\n\nBody\n\n  **added**\n"
            );
            assert_eq!(this.model.rows.len(), 2);
            this.composer
                .update(cx, |input, cx| input.set_value("next draft", window, cx));
            this.submit(window, cx);
            assert_eq!(this.composer.read(cx).value(), "next draft");
            assert_eq!(this.model.capture().appends.pending.len(), 1);
            this.select(1, window, cx);
            assert_eq!(this.detail.as_ref().unwrap().source, "");
            this.change_destination(Destination::Home, window, cx);
            this.composer
                .update(cx, |input, cx| input.set_selected_range(2..6, cx));
        });
        view.update(cx, |this, cx| this.select(2, window, cx));
        window.render_frame(cx);
        window.click("copy-detail", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "# Original\n\nBody",
            "optimistic detail never replaces canonical toolbar Copy"
        );
        view.update(cx, |this, cx| this.close_detail(window, cx));
        window.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    drop(view);
    drop(writer);
    // Services complete while no window owns a watch or view.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !services
        .operations
        .lock()
        .unwrap()
        .iter()
        .all(tokio::task::AbortHandle::is_finished)
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let (window, view) = open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            assert_eq!(this.composer.read(cx).value(), "next draft");
            assert_eq!(this.composer.read(cx).selected_range(), 2..6);
            assert!(this.model.capture().appends.pending.is_empty());
            this.select(2, window, cx);
            assert_eq!(
                this.detail.as_ref().unwrap().source,
                "# Original\n\nBody\n\n  **added**\n"
            );
            this.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
        });
        window.render_frame(cx);
        window.click("copy-detail", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "# Original\n\nBody\n\n  **added**\n"
        );
        view.update(cx, |this, cx| {
            this.select(1, window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value(" empty-body append \n", window, cx)
            });
            this.submit(window, cx);
            assert_eq!(
                this.detail.as_ref().unwrap().source,
                " empty-body append \n"
            );
        });
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.capture().appends.pending.is_empty())
    });
    runtime.block_on(async {
        let reader = host.db.reader().await.unwrap();
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        let values = reader
            .query_row(
                "SELECT status,title,metadata,content FROM notes WHERE short_id=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            values,
            (
                "ready".into(),
                "Original title".into(),
                "{\"created_by_ai\":true}".into(),
                " empty-body append \n".into()
            )
        );
    });
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn append_possible_after_write_retains_truthful_recovery(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = fixture(&runtime, root.path());
    // Existing typed metadata read fails, but content resolution/update and Today watch succeed.
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET status=NULL WHERE short_id=2", [])
            .unwrap();
    });
    let services = services(&host, &runtime);
    cx.update(gpui_kit::init);
    let (window, view) = open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.select(2, window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value("possible write", window, cx)
            });
            this.submit(window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("new typing", window, cx));
            this.select(1, window, cx);
        });
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).model.capture().appends.recovery.is_empty())
    });
    cx.update_window(window.into(), |_, _window, cx| {
        view.update(cx, |this, cx| {
            assert_eq!(this.composer.read(cx).value(), "new typing");
            assert_eq!(this.detail.as_ref().unwrap().source, "");
            let capture = this.model.capture();
            let recovery = &capture.appends.recovery[0];
            assert!(recovery.uncertain);
            assert_eq!(recovery.uuid, "00000000-0000-4000-8000-000000000002");
            assert_eq!(recovery.text, "possible write");
        });
    })
    .unwrap();
    runtime.block_on(async {
        let reader = host.db.reader().await.unwrap();
        assert_eq!(
            reader
                .query_row("SELECT content FROM notes WHERE short_id=2", [], |r| r
                    .get::<_, String>(
                    0
                ))
                .unwrap(),
            "# Original\n\nBody\n\npossible write"
        );
    });
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn append_parse_follows_its_own_reader_end(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = fixture(&runtime, root.path());
    let services = services(&host, &runtime);
    cx.update(gpui_kit::init);
    let (window, view) = open(cx, services);
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let continuation = format!(
        "{}\n\nAPPENDED END",
        "Long paragraph 中文.\n\n".repeat(1000)
    );
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.select(2, window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value(continuation, window, cx));
            this.submit(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let reading = view.read(cx).detail.as_ref().unwrap();
            reading
                .state
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("APPENDED END")
        })
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        let reading = view.read(cx).detail.as_ref().unwrap();
        assert!(
            reading.scroll.offset().y < px(0.),
            "offset {:?}, max {:?}, subscription {}",
            reading.scroll.offset(),
            reading.scroll.max_offset(),
            reading.append_scroll.is_some()
        );
    })
    .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.select(1, window, cx);
            assert_eq!(this.detail.as_ref().unwrap().scroll.offset().y, px(0.));
        });
        window.remove_window();
    })
    .unwrap();
    drop(view);
    runtime.block_on(host.shutdown());
}
