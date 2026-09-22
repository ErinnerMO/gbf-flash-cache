//! C ABI used by platform hosts. Calls for a handle must be serialized; close consumes it.
use crate::application::{Fields, Service};
use std::{
    ffi::{c_char, CStr, CString},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Mutex,
};

pub struct Core {
    service: Mutex<Service>,
    runtime: tokio::runtime::Runtime,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    op: String,
    #[serde(default)]
    args: Fields,
}
fn message(result: Result<Fields, String>) -> *mut c_char {
    let value = match result {
        Ok(fields) => serde_json::json!({"ok":true,"fields":fields}),
        Err(error) => serde_json::json!({"ok":false,"error":error}),
    };
    CString::new(value.to_string()).unwrap().into_raw()
}
unsafe fn input<'a>(text: *const c_char) -> Result<&'a str, String> {
    if text.is_null() {
        return Err("缺少参数".into());
    }
    let value = unsafe { CStr::from_ptr(text) }
        .to_str()
        .map_err(|_| "参数必须为 UTF-8")?;
    if value.len() > 1024 * 1024 {
        return Err("参数过大".into());
    }
    Ok(value)
}
/// # Safety
/// `home` is a valid NUL-terminated UTF-8 path; `error` points to writable pointer storage.
/// Free a returned error with `gbf_core_free_string`; close a successful handle exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gbf_core_open(home: *const c_char, error: *mut *mut c_char) -> *mut Core {
    unsafe { open(home, error, None) }
}
// Same pointer contract as gbf_core_open; JNI supplies Android platform trust roots.
pub(super) unsafe fn open(
    home: *const c_char,
    error: *mut *mut c_char,
    roots: Option<Vec<Vec<u8>>>,
) -> *mut Core {
    if error.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        *error = std::ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Core, String> {
        let home = std::path::PathBuf::from(unsafe { input(home)? });
        let service = match roots {
            Some(roots) => Service::open_with_roots(home, roots)?,
            None => Service::open(home)?,
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .max_blocking_threads(8)
            .build()
            .map_err(|_| "无法启动核心线程")?;
        Ok(Core {
            service: Mutex::new(service),
            runtime,
        })
    }))
    .unwrap_or_else(|_| Err("核心初始化异常".into()));
    match result {
        Ok(core) => Box::into_raw(Box::new(core)),
        Err(e) => {
            unsafe {
                *error = message(Err(e));
            }
            std::ptr::null_mut()
        }
    }
}
/// # Safety
/// `core` is a live handle from `gbf_core_open`; `json` is a valid NUL-terminated UTF-8 string.
/// Do not call concurrently with close. Free the response with `gbf_core_free_string`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gbf_core_command(core: *mut Core, json: *const c_char) -> *mut c_char {
    message(
        catch_unwind(AssertUnwindSafe(|| -> Result<Fields, String> {
            let core = unsafe { core.as_ref() }.ok_or("核心未启动")?;
            let command: Command =
                serde_json::from_str(unsafe { input(json)? }).map_err(|_| "无效命令")?;
            let mut service = core.service.lock().map_err(|_| "核心状态异常")?;
            core.runtime
                .block_on(service.command(&command.op, command.args))
        }))
        .unwrap_or_else(|_| Err("核心执行异常".into())),
    )
}
/// # Safety
/// `core` is a live handle from `gbf_core_open` and no other calls are using it.
/// This consumes the handle; free the response with `gbf_core_free_string`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gbf_core_close(core: *mut Core) -> *mut c_char {
    message(
        catch_unwind(AssertUnwindSafe(|| -> Result<Fields, String> {
            if core.is_null() {
                return Err("核心未启动".into());
            }
            let core = unsafe { Box::from_raw(core) };
            {
                let mut service = core.service.lock().map_err(|_| "核心状态异常")?;
                core.runtime.block_on(service.close())?;
            }
            Ok(Fields::new())
        }))
        .unwrap_or_else(|_| Err("核心关闭异常".into())),
    )
}
/// # Safety
/// `text` is null or an outstanding response/error allocated by this library; free it once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gbf_core_free_string(text: *mut c_char) {
    if !text.is_null() {
        drop(unsafe { CString::from_raw(text) });
    }
}
