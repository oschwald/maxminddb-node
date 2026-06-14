use napi_derive::napi;

#[napi(js_name = "nativeVersion")]
pub fn native_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

