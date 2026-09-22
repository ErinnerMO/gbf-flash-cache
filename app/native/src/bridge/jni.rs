//! Android's foreground service owns the same C ABI handle as the Windows host.
use super::ffi;
use jni::{
    objects::{JByteArray, JClass, JObjectArray},
    sys::{jbyteArray, jlong},
    JNIEnv,
};
use std::ffi::{CStr, CString};

fn input(env: &mut JNIEnv, bytes: JByteArray) -> Option<CString> {
    let bytes = env.convert_byte_array(bytes).ok()?;
    match CString::new(bytes) {
        Ok(value) => Some(value),
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalArgumentException", "Invalid native input");
            None
        }
    }
}
unsafe fn output(env: &mut JNIEnv, text: *mut std::ffi::c_char) -> jbyteArray {
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    let result = env
        .byte_array_from_slice(bytes)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut());
    unsafe { ffi::gbf_core_free_string(text) };
    result
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_open(
    mut env: JNIEnv,
    _: JClass,
    path: JByteArray,
    roots: JObjectArray,
) -> jlong {
    let Some(path) = input(&mut env, path) else {
        return 0;
    };
    let roots = (|| -> jni::errors::Result<Vec<Vec<u8>>> {
        let count = env.get_array_length(&roots)?;
        let mut certificates = Vec::with_capacity(count as usize);
        for i in 0..count {
            let cert = env.get_object_array_element(&roots, i)?;
            let cert = env.auto_local(JByteArray::from(cert));
            certificates.push(env.convert_byte_array(&*cert)?);
        }
        Ok(certificates)
    })();
    let Ok(roots) = roots else {
        if !env.exception_check().unwrap_or(true) {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "无法读取 Android 信任证书",
            );
        }
        return 0;
    };
    let mut error = std::ptr::null_mut();
    let core = unsafe { ffi::open(path.as_ptr(), &mut error, Some(roots)) };
    if core.is_null() && !error.is_null() {
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::gbf_core_free_string(error) };
        let _ = env.throw_new("java/lang/IllegalStateException", message);
    }
    core as jlong
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_command(
    mut env: JNIEnv,
    _: JClass,
    handle: jlong,
    json: JByteArray,
) -> jbyteArray {
    let Some(json) = input(&mut env, json) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let text = ffi::gbf_core_command(handle as *mut ffi::Core, json.as_ptr());
        output(&mut env, text)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_close(
    mut env: JNIEnv,
    _: JClass,
    handle: jlong,
) -> jbyteArray {
    unsafe {
        let text = ffi::gbf_core_close(handle as *mut ffi::Core);
        output(&mut env, text)
    }
}
