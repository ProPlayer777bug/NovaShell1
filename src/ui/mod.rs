//! WebView bridge: the single channel between the Rust core and the
//! JavaScript UI.
//!
//! * JS -> Rust: `webkit.messageHandlers.novashell.postMessage(JSON)`
//! * Rust -> JS: `window.novaShellDispatch(JSON)`

use webkit6::javascriptcore::Value as JsValue;
use webkit6::prelude::WebViewExt;

/// Evaluate a script with the fire-and-forget callback.
pub fn eval_js(webview: &webkit6::WebView, script: &str) {
    webview.evaluate_javascript(
        script,
        None,
        None,
        None::<&gtk4::gio::Cancellable>,
        move |result| {
            if let Err(e) = result {
                log::warn!("js eval error: {e}");
            }
        },
    );
}

/// Dispatch a JSON payload into the UI.
pub fn dispatch(webview: &webkit6::WebView, payload: &serde_json::Value) {
    let js = format!(
        "window.novaShellDispatch && window.novaShellDispatch({});",
        payload
    );
    eval_js(webview, &js);
}

/// Decode a script-message handler value into a JSON value.
pub fn decode_message(value: &JsValue) -> Option<serde_json::Value> {
    let s = value.to_str().to_string();
    serde_json::from_str(&s).ok()
}

/// Namespace used for the script message handler and the JS dispatcher.
pub fn namespace() -> &'static str {
    "novashell"
}