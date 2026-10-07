//! The vault key wrapped by the Android Keystore, through the Kotlin
//! `VaultKeystore` object. Called over JNI on the current thread (not the
//! plugin bridge, which hops to the UI thread and could deadlock when the
//! vault is first unlocked from a command running there).

use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use zeroize::Zeroizing;

use super::vault::KeyWrap;

const CLASS: &str = "trade.autologin.autologin.VaultKeystore";
const LOCAL_REFS: i32 = 16;

pub struct AndroidKeystore;

impl KeyWrap for AndroidKeystore {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, String> {
        call_bytes("wrap", key)
    }

    fn unwrap(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        call_bytes("unwrap", blob).map(Zeroizing::new)
    }
}

/// Calls `VaultKeystore.<method>(byte[]): byte[]`.
fn call_bytes(method: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let context = ndk_context::android_context();
    // SAFETY: tao initialises the Android context with a valid JavaVM pointer
    // and a global reference to the Application before any app code runs.
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let application = unsafe { JObject::from_raw(context.context().cast()) };
    // A local frame frees the call's references even on a thread that stays
    // attached to the JVM (they'd otherwise live until it detaches).
    let result = env.with_local_frame(LOCAL_REFS, |env| invoke(env, &application, method, input));
    result.map_err(|error| java_error(&mut env, error))
}

fn invoke(env: &mut JNIEnv, application: &JObject, method: &str, input: &[u8]) -> jni::errors::Result<Vec<u8>> {
    // App classes aren't visible to FindClass off the main thread; use the
    // application's class loader.
    let loader = env.call_method(application, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
    let name = env.new_string(CLASS)?;
    let class = env
        .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
        .l()?;
    let input = env.byte_array_from_slice(input)?;
    let output = env
        .call_static_method(JClass::from(class), method, "([B)[B", &[JValue::Object(&input)])?
        .l()?;
    env.convert_byte_array(JByteArray::from(output))
}

/// Clears a pending Java exception (it would abort the next JNI call) and
/// returns its message.
fn java_error(env: &mut JNIEnv, error: jni::errors::Error) -> String {
    if !env.exception_check().unwrap_or(false) {
        return error.to_string();
    }
    let exception = env.exception_occurred();
    let _ = env.exception_clear();
    exception
        .ok()
        .and_then(|e| env.call_method(&e, "toString", "()Ljava/lang/String;", &[]).ok())
        .and_then(|text| text.l().ok())
        .and_then(|text| env.get_string(&text.into()).ok().map(String::from))
        .unwrap_or_else(|| error.to_string())
}
