//! Window-scoped email/code requests under the startup owner's profile lock.
use flicknote_auth::client::GoTrueClient;
use flicknote_core::config::Config;
use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::component::{
    Theme,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use gpui_kit::{Context, Entity, Render, Subscription, Task, Window, div, prelude::*, px};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone)]
pub(crate) struct LoginHandle(mpsc::Sender<Request>);
struct Request {
    email: String,
    code: Option<String>,
    response: oneshot::Sender<Result<(), String>>,
    closed: watch::Receiver<bool>,
}

pub(crate) async fn authenticate(
    config: &Config,
    mut quit: watch::Receiver<bool>,
    show: impl FnOnce(LoginHandle) + Send,
) -> Result<(), String> {
    let client = GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    );
    let (sender, mut requests) = mpsc::channel::<Request>(1);
    show(LoginHandle(sender));
    loop {
        if *quit.borrow() {
            return Err("Sign-in cancelled".into());
        }
        let mut request = tokio::select! {
            biased;
            _ = quit.changed() => return Err("Sign-in cancelled".into()),
            request = requests.recv() => request.ok_or("Sign-in closed")?,
        };
        if *request.closed.borrow() {
            continue;
        }
        let verifying = request.code.is_some();
        let operation = async {
            match &request.code {
                Some(code) => client.verify_otp(&request.email, code).await.map(|_| ()),
                None => client.sign_in_with_otp(&request.email).await,
            }
        };
        let result = tokio::select! {
            biased;
            _ = quit.changed() => return Err("Sign-in cancelled".into()),
            _ = request.closed.changed() => continue,
            _ = request.response.closed() => continue,
            result = tokio::time::timeout(std::time::Duration::from_secs(30), operation) => {
                match result {
                    Ok(result) => result.map_err(|_| "Could not complete sign-in. Try again.".into()),
                    Err(_) => Err("Sign-in request timed out. Try again.".into()),
                }
            }
        };
        let accepted = result.is_ok() && verifying;
        if *quit.borrow() {
            return Err("Sign-in cancelled".into());
        }
        if !*request.closed.borrow() && request.response.send(result).is_ok() && accepted {
            return Ok(());
        }
    }
}

pub(crate) struct LoginPane {
    handle: LoginHandle,
    email: Entity<InputState>,
    code: Entity<InputState>,
    code_sent: bool,
    busy: bool,
    error: Option<String>,
    request: Option<Task<()>>,
    closed: watch::Receiver<bool>,
    _on_closed: Subscription,
}
impl LoginPane {
    pub(crate) fn new(handle: LoginHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| InputState::new(window, cx).placeholder("Email address"));
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("Verification code"));
        email.update(cx, |input, cx| input.focus(window, cx));
        let id = window.window_handle().window_id();
        let (cancel, closed) = watch::channel(false);
        let on_closed = cx.on_window_closed(move |_, closed_id| {
            if closed_id == id {
                cancel.send_replace(true);
            }
        });
        Self {
            handle,
            email,
            code,
            code_sent: false,
            busy: false,
            error: None,
            request: None,
            closed,
            _on_closed: on_closed,
        }
    }
    fn send(&mut self, verifying: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let email = self.email.read(cx).value().trim().to_owned();
        let code = verifying.then(|| self.code.read(cx).value().trim().to_owned());
        if email.is_empty() || code.as_ref().is_some_and(String::is_empty) {
            self.error = Some("Enter your email and verification code.".into());
            cx.notify();
            return;
        }
        let (response, result) = oneshot::channel();
        if self
            .handle
            .0
            .try_send(Request {
                email,
                code,
                response,
                closed: self.closed.clone(),
            })
            .is_err()
        {
            self.error = Some("Sign-in is unavailable. Quit and reopen this profile.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        // Window-close and receiver-drop signals cancel the coordinator's HTTP future.
        self.request = Some(cx.spawn_in(window, async move |entity, cx| {
            let result = result.await;
            let _updated = entity.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(())) if !verifying => {
                        this.code_sent = true;
                        this.code.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                            input.focus(window, cx);
                        });
                    }
                    Ok(Err(error)) => this.error = Some(error),
                    Err(_) => {
                        this.error = Some("Sign-in cancelled. Quit and reopen this profile.".into())
                    }
                    Ok(Ok(())) => {}
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for LoginPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::global(cx).color_tokens();
        div()
            .id("login-workspace")
            .test_support()
            .size_full()
            .bg(p.background)
            .text_color(p.foreground)
            .font_family(".SystemUIFont")
            .text_size(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("login-form")
                    .test_support()
                    .w(px(360.))
                    .p(px(20.))
                    .border_l_1()
                    .border_color(p.border)
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(div().text_size(px(16.)).child("Sign in to FlickNote"))
                    .child(
                        div()
                            .text_color(p.secondary_foreground)
                            .child("Use your email to sign in."),
                    )
                    .when(!self.code_sent, |d| {
                        d.child(Input::new(&self.email).disabled(self.busy)).child(
                            Button::new("send-code")
                                .label(if self.busy { "Sending…" } else { "Send code" })
                                .primary()
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.send(false, window, cx)),
                                ),
                        )
                    })
                    .when(self.code_sent, |d| {
                        d.child(self.email.read(cx).value())
                            .child(Input::new(&self.code).disabled(self.busy))
                            .child(
                                Button::new("verify-code")
                                    .label(if self.busy {
                                        "Signing in…"
                                    } else {
                                        "Verify and open Today"
                                    })
                                    .primary()
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send(true, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("resend-code")
                                    .label("Resend code")
                                    .ghost()
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("change-email")
                                    .label("Change email")
                                    .ghost()
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.code_sent = false;
                                        this.error = None;
                                        this.email.update(cx, |input, cx| input.focus(window, cx));
                                        cx.notify();
                                    })),
                            )
                    })
                    .children(self.error.clone().map(|error| {
                        div()
                            .id("login-error")
                            .test_support()
                            .text_size(px(12.))
                            .text_color(p.destructive)
                            .child(error)
                    })),
            )
    }
}

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
