//! Main-thread AppKit status item. Native callbacks only enqueue GPUI actions.
#![allow(unsafe_code)]
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly, Message, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject, sel,
};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityCustomAction, NSApplication, NSEventMask, NSEventType, NSImage,
    NSMenu, NSMenuItem, NSStatusBar, NSStatusBarButton, NSStatusItem,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSData, NSObject, NSObjectProtocol, NSPoint, NSSize, NSString,
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Open,
    Quit,
}

struct TargetState {
    actions: UnboundedSender<Action>,
    menu: Retained<NSMenu>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. All state and callbacks
    // are main-thread-only, and the Tray retains the target until it detaches.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetState]
    struct Target;
    unsafe impl NSObjectProtocol for Target {}
    impl Target {
        #[unsafe(method(clicked:))]
        fn clicked(&self, button: &NSStatusBarButton) {
            // Native menu tracking can process Quit before this callback returns.
            // Keep the callback receiver and sender alive even if Tray is removed.
            let target = self.retain();
            let button = button.retain();
            let event = NSApplication::sharedApplication(self.mtm()).currentEvent();
            if event.is_some_and(|event| event.r#type() == NSEventType::RightMouseUp) {
                let _shown = target.ivars().menu.popUpMenuPositioningItem_atLocation_inView(
                    None, NSPoint::new(0., 0.), Some(&button));
            } else {
                // Keyboard/AX activation has no right mouse event and opens too.
                target.send(Action::Open);
            }
        }
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSObject) { self.send(Action::Quit); }
        #[unsafe(method(accessibleOpen:))]
        fn accessible_open(&self, _action: &NSAccessibilityCustomAction) -> bool {
            self.send(Action::Open);
            true
        }
        #[unsafe(method(accessibleQuit:))]
        fn accessible_quit(&self, _action: &NSAccessibilityCustomAction) -> bool {
            self.send(Action::Quit);
            true
        }
    }
);
impl Target {
    fn send(&self, action: Action) {
        // A closed receiver means GPUI has already shut down.
        let _sent = self.ivars().actions.send(action);
    }
}

pub(crate) struct Tray {
    item: Retained<NSStatusItem>,
    target: Retained<Target>,
}
impl Tray {
    pub(crate) fn new(actions: UnboundedSender<Action>) -> anyhow::Result<Self> {
        let mtm = MainThreadMarker::new().expect("tray created on AppKit main thread");
        let data = NSData::with_bytes(include_bytes!("../assets/tray/template.png"));
        let image = NSImage::initWithData(NSImage::alloc(), &data)
            .ok_or_else(|| anyhow::anyhow!("Could not load FlickNote menu bar icon"))?;
        image.setSize(NSSize::new(18., 18.));
        image.setTemplate(true);
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        // SAFETY: initialized NSObject subclass, selectors below match their
        // target method signatures; target is retained through every callback.
        let this = Target::alloc(mtm).set_ivars(TargetState { actions, menu });
        let target: Retained<Target> = unsafe { msg_send![super(this), init] };
        let quit = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str("Quit FlickNote"),
                Some(sel!(quit:)),
                &NSString::from_str("q"),
            )
        };
        unsafe {
            quit.setTarget(Some(&target));
        }
        target.ivars().menu.addItem(&quit);
        let accessible_open = unsafe {
            NSAccessibilityCustomAction::initWithName_target_selector(
                NSAccessibilityCustomAction::alloc(),
                &NSString::from_str("Open FlickNote"),
                ProtocolObject::from_ref(&*target),
                sel!(accessibleOpen:),
            )
        };
        let accessible_quit = unsafe {
            NSAccessibilityCustomAction::initWithName_target_selector(
                NSAccessibilityCustomAction::alloc(),
                &NSString::from_str("Quit FlickNote"),
                ProtocolObject::from_ref(&*target),
                sel!(accessibleQuit:),
            )
        };
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(28.);
        let tray = Self { item, target };
        let button = tray
            .item
            .button(mtm)
            .ok_or_else(|| anyhow::anyhow!("No menu bar button"))?;
        button.setImage(Some(&image));
        button.setToolTip(Some(&NSString::from_str("FlickNote")));
        button.setAccessibilityLabel(Some(&NSString::from_str("FlickNote")));
        button.setAccessibilityHelp(Some(&NSString::from_str(
            "Open FlickNote. Right-click for Quit FlickNote.",
        )));
        button.setAccessibilityCustomActions(Some(&NSArray::from_retained_slice(&[
            accessible_open,
            accessible_quit,
        ])));
        unsafe {
            button.setTarget(Some(&tray.target));
            button.setAction(Some(sel!(clicked:)));
        }
        let _previous = button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        Ok(tray)
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        // Detach weak targets before releasing their Rust state. No callbacks
        // may outlive the status item, including during application shutdown.
        if let Some(button) = self.item.button(self.target.mtm()) {
            unsafe {
                button.setAction(None);
                button.setTarget(None);
            }
            if let Some(actions) = button.accessibilityCustomActions() {
                for action in &actions {
                    unsafe {
                        action.setTarget(None);
                        action.setSelector(None);
                    }
                }
            }
            button.setAccessibilityCustomActions(None);
        }
        self.target.ivars().menu.cancelTracking();
        for item in &self.target.ivars().menu.itemArray() {
            unsafe {
                item.setTarget(None);
                item.setAction(None);
            }
        }
        self.target.ivars().menu.removeAllItems();
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}

#[cfg(test)]
#[path = "tray_tests.rs"]
pub(crate) mod tests;
