use super::*;
use std::sync::Mutex;

fn picture() -> BrowserPicture {
    BrowserPicture {
        bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
        media_type: "image/png",
        after: "open".into(),
        taken_at_ms: 1,
    }
}

#[test]
fn a_registered_sink_gets_each_picture_and_a_refusal_is_not_fatal() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let into = seen.clone();
    register_browser_picture_sink("test-pictures-ok", move |p: &BrowserPicture| {
        into.lock().unwrap().push(p.after.clone());
        Ok(())
    });
    register_browser_picture_sink("test-pictures-refuses", |_: &BrowserPicture| {
        Err("disk full".to_owned())
    });
    assert!(wanted());
    assert!(deliver(&picture()) >= 1);
    assert_eq!(seen.lock().unwrap().as_slice(), ["open"]);
    assert!(unregister_browser_picture_sink("test-pictures-ok"));
    assert!(unregister_browser_picture_sink("test-pictures-refuses"));
    assert!(!unregister_browser_picture_sink("test-pictures-ok"));
}
