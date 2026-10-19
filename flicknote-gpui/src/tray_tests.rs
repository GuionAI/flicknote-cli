//! Explicit main-thread native contract probe, invoked by the owned scratch runner.
//! It does not prove physical mouse delivery, menu pixels or VoiceOver navigation.
use super::*;

#[allow(dead_code)] // Explicit main-thread runner; libtest workers cannot own AppKit.
pub(crate) fn native_contract() {
    let mtm = MainThreadMarker::new().expect("native probe requires process main thread");
    let app = NSApplication::sharedApplication(mtm);
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    objc2::rc::autoreleasepool(|_| {
        let tray = Tray::new(send).unwrap();
        let button = tray.item.button(mtm).unwrap();
        assert!(
            tray.item.menu(mtm).is_none(),
            "left click must not automatically pop a menu"
        );
        assert_eq!(
            button.accessibilityLabel().unwrap().to_string(),
            "FlickNote"
        );
        let image = button.image().unwrap();
        assert!(image.isTemplate());
        assert_eq!(image.size(), NSSize::new(18., 18.));
        assert_eq!(tray.target.ivars().menu.numberOfItems(), 1);
        let quit = tray.target.ivars().menu.itemAtIndex(0).unwrap();
        assert_eq!(quit.title().to_string(), "Quit FlickNote");
        // Real AppKit target/action invocation checks the registered Objective-C ABI.
        // SAFETY: the retained target implements the registered clicked: selector.
        unsafe {
            button.performClick(None);
        }
        assert_eq!(receive.try_recv().unwrap(), Action::Open);
        assert!(receive.try_recv().is_err());
        tray.target.ivars().menu.performActionForItemAtIndex(0);
        assert_eq!(receive.try_recv().unwrap(), Action::Quit);
        assert!(receive.try_recv().is_err());
        let custom = button.accessibilityCustomActions().unwrap();
        let open = custom.objectAtIndex(0);
        assert_eq!(open.name().to_string(), "Open FlickNote");
        let accepted: bool = unsafe { msg_send![&tray.target, accessibleOpen: &*open] };
        assert!(accepted);
        assert_eq!(receive.try_recv().unwrap(), Action::Open);
        let action = custom.objectAtIndex(1);
        assert_eq!(action.name().to_string(), "Quit FlickNote");
        // NSAccessibility invokes this selector with its action argument and BOOL return.
        let accepted: bool = unsafe { msg_send![&tray.target, accessibleQuit: &*action] };
        assert!(accepted);
        assert_eq!(receive.try_recv().unwrap(), Action::Quit);
        drop(tray);
        assert!(button.action().is_none());
        assert!(button.target().is_none());
        assert!(button.accessibilityCustomActions().is_none());
        assert!(quit.target().is_none(), "AppKit weak menu target released");
    });
    assert!(receive.is_closed());
    drop(app);
}
