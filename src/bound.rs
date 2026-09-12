use crate::NativeReader;
use napi::{
    bindgen_prelude::{Env, JsValue, Reference, ToNapiValue, Unknown},
    sys, JsString, Result,
};

// The callback creates this value and returns it to Node in the same handle
// scope. It never stores the handle or exposes it outside that callback.
struct CallbackValue(sys::napi_value);

impl ToNapiValue for CallbackValue {
    unsafe fn to_napi_value(_: sys::napi_env, value: Self) -> Result<sys::napi_value> {
        Ok(value.0)
    }
}

pub(crate) fn bind_get(env: &Env, reader: Reference<NativeReader>) -> Result<Unknown<'_>> {
    // Reference validates the native type when the function is bound and keeps
    // that reader alive until the function is collected. NativeReader must keep
    // all exposed methods on shared receivers while this callback can borrow
    // it. get() protects mutable state with RefCell, just like the other reader
    // methods. No native reference comes from `this`
    // or from the callback arguments, so generic per-call alias tracking is
    // unnecessary here. Keep the function on the JavaScript Reader wrapper,
    // never on NativeReader itself, to avoid a reference cycle.
    let function = env.create_function_from_closure::<(), CallbackValue, _>("get", move |ctx| {
        let ip = ctx.first_arg::<JsString>()?;
        reader
            .get(ctx.env, ip)
            .map(|value| CallbackValue(value.raw()))
    })?;
    function.into_unknown(env)
}
