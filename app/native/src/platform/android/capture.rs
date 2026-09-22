//! Android-only TUN adapter. Cache routing and upstream configuration stay unchanged.
#[cfg(target_os = "android")]
use jni::{
    objects::{JClass, JString},
    sys::{jint, jstring},
    JNIEnv,
};
use std::sync::{Mutex, Arc};
#[cfg(target_os = "android")]
use super::forwarder;
#[cfg(not(target_os = "android"))]
use crate::android_forwarder_tests as forwarder;
use gbf_flash_cache_core::trace::Trace;
use tokio::{runtime::Runtime, task::JoinHandle};
use tokio_util::sync::CancellationToken;

struct Capture {
    device: Tun,
    runtime: Runtime,
    task: JoinHandle<Result<(), String>>,
    cancel: CancellationToken,
    trace: Arc<Trace>,
}
static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

fn start(fd: i32, port: i32, route: forwarder::udp::Route, logs: &str) -> Result<(), String> {
    if fd < 0 || !(1..=65535).contains(&port) {
        return Err("无效的系统代理参数".into());
    }
    let mut state = CAPTURE.lock().map_err(|_| "系统代理状态不可用")?;
    if state.is_some() {
        return Err("系统代理已启动".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let device = {
        let _guard = runtime.enter();
        use std::os::fd::BorrowedFd;
        // The Java service owns fd through this call; the task takes its own duplicate.
        let owned = unsafe { BorrowedFd::borrow_raw(fd) }
            .try_clone_to_owned()
            .map_err(|e| e.to_string())?;
        Tun(Arc::new(Mutex::new(Some(tokio::io::unix::AsyncFd::new(std::fs::File::from(owned)).map_err(|e| e.to_string())?))))
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let trace = Trace::open(&std::path::Path::new(logs).join(format!("{stamp}-tun.jsonl"))).map_err(|e| e.to_string())?;
    trace.event("TUN_START", None, serde_json::json!({"version":env!("CARGO_PKG_VERSION")}));
    let trace_task = trace.clone();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task_device = device.clone();
    let task = runtime.spawn(async move {
        forwarder::run(task_device, port as u16, route, token, Some(trace_task))
            .await
            .map_err(|e| e.to_string())
    });
    *state = Some(Capture {
        device,
        runtime,
        task,
        cancel,
        trace,
    });
    Ok(())
}
fn stop() {
    let state = CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(capture) = state {
        capture.stop();
    }
}
impl Capture {
    fn stop(self) {
        let Capture { device, runtime, mut task, cancel, trace } = self;
        // Release our duplicated TUN fd even if a task's destructor blocks.
        device.close();
        cancel.cancel();
        task.abort();
        runtime.block_on(async {
            let timed_out = tokio::time::timeout(std::time::Duration::from_secs(2), &mut task)
                .await.is_err();
            trace.event("TUN_STOP", None, serde_json::json!({"cleanup_timed_out": timed_out}));
            // Log flushing must not keep Android's stop callback pending either.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), trace.close()).await;
        });
        runtime.shutdown_timeout(std::time::Duration::from_millis(500));
    }
}
#[cfg(target_os = "android")]
fn route(env: &mut JNIEnv, settings: &JString) -> Result<forwarder::udp::Route, String> {
    let json: String = env.get_string(settings).map_err(|e| e.to_string())?.into();
    let fields = serde_json::from_str(&json).map_err(|_| "无效的连接配置")?;
    forwarder::udp::from_fields(&fields).map_err(|e| e.to_string())
}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_captureCheck(
    mut env: JNIEnv,
    _: JClass,
    settings: JString,
) {
    let result = (|| -> Result<(), String> {
        let route = route(&mut env, &settings)?;
        if matches!(route, forwarder::udp::Route::Socks { .. }) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            runtime
                .block_on(route.open(None))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = env.throw_new("java/lang/IllegalStateException", error);
    }
}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_captureStart(
    mut env: JNIEnv,
    _: JClass,
    fd: jint,
    port: jint,
    settings: JString,
) {
    let result = (|| -> Result<(), String> {
        let json: String = env.get_string(&settings).map_err(|e| e.to_string())?.into();
        let fields: std::collections::BTreeMap<String,String> = serde_json::from_str(&json).map_err(|_| "无效的连接配置")?;
        let route = forwarder::udp::from_fields(&fields).map_err(|e| e.to_string())?;
        start(fd, port, route, fields.get("logsPath").ok_or("缺少日志目录")?)
    })();
    if let Err(error) = result {
        let _ = env.throw_new("java/lang/IllegalStateException", error);
    }
}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_captureStop(_: JNIEnv, _: JClass) {
    stop();
}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_gbfcache_flashcache_NativeCore_captureStatus(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    let state = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
    let message = match state.as_ref() {
        Some(capture) if !capture.task.is_finished() => "",
        _ => "系统代理转发已停止，请重新开启",
    };
    env.new_string(message)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// Android supplies an already nonblocking TUN fd; no device creation/configuration library is needed.
#[derive(Clone)]
struct Tun(Arc<Mutex<Option<tokio::io::unix::AsyncFd<std::fs::File>>>>);
impl Tun {
    fn close(&self) { self.0.lock().unwrap_or_else(|e| e.into_inner()).take(); }
}
impl tokio::io::AsyncRead for Tun {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::{
            io::Read,
            task::{ready, Poll},
        };
        let device = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(device) = device.as_ref() else {
            return Poll::Ready(Err(std::io::ErrorKind::NotConnected.into()));
        };
        loop {
            let mut guard = ready!(device.poll_read_ready(cx))?;
            match guard.try_io(|fd| (&*fd.get_ref()).read(buf.initialize_unfilled())) {
                Ok(Ok(size)) => {
                    buf.advance(size);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) => return Poll::Ready(Err(error)),
                Err(_) => continue,
            }
        }
    }
}
impl tokio::io::AsyncWrite for Tun {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        use std::{
            io::Write,
            task::{ready, Poll},
        };
        let device = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(device) = device.as_ref() else {
            return Poll::Ready(Err(std::io::ErrorKind::NotConnected.into()));
        };
        loop {
            let mut guard = ready!(device.poll_write_ready(cx))?;
            match guard.try_io(|fd| (&*fd.get_ref()).write(buf)) {
                Ok(result) => return Poll::Ready(result),
                Err(_) => continue,
            }
        }
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{io::Read, os::fd::{AsRawFd, OwnedFd}, os::unix::net::UnixStream, time::{Duration, Instant}};

    #[test]
    fn stop_releases_tun_and_returns_even_when_task_drop_is_blocked() {
        // Use a nonblocking socket fd to test the real TUN owner without a VPN/device.
        let home = tempfile::tempdir().unwrap();
        for _ in 0..3 {
            let (device, mut peer) = UnixStream::pair().unwrap();
            device.set_nonblocking(true).unwrap();
            start(device.as_raw_fd(), 8765, forwarder::udp::Route::Direct, home.path().to_str().unwrap()).unwrap();
            drop(device);
            stop();
            assert!(CAPTURE.lock().unwrap().is_none());
            peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        }

        struct BlockedDrop {
            _device: Tun,
            release: std::sync::mpsc::Receiver<()>,
            finished: std::sync::mpsc::Sender<()>,
        }
        impl Drop for BlockedDrop {
            fn drop(&mut self) {
                tokio::task::block_in_place(|| {
                    let _ = self.release.recv_timeout(Duration::from_secs(8));
                });
                let _ = self.finished.send(());
            }
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let (fd, mut peer) = UnixStream::pair().unwrap();
        fd.set_nonblocking(true).unwrap();
        let device = {
            let _entered = runtime.enter();
            let fd: OwnedFd = fd.into();
            Tun(Arc::new(Mutex::new(Some(tokio::io::unix::AsyncFd::new(std::fs::File::from(fd)).unwrap()))))
        };
        let (release, receiver) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let guard = BlockedDrop { _device: device.clone(), release: receiver, finished };
        let (ready, entered) = std::sync::mpsc::channel();
        let task = runtime.spawn(async move {
            let _guard = guard;
            ready.send(()).unwrap();
            std::future::pending::<Result<(), String>>().await
        });
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let trace = Trace::open(&home.path().join("blocked.jsonl")).unwrap();
        let capture = Capture { device, runtime, task, cancel: CancellationToken::new(), trace };
        let began = Instant::now();
        capture.stop();
        let elapsed = began.elapsed();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let closed = peer.read(&mut [0]);
        // Let the artificial blocked destructor exit even if an assertion fails.
        let _ = release.send(());
        done.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(elapsed < Duration::from_secs(4), "stop took {elapsed:?}");
        assert_eq!(closed.unwrap(), 0);
        assert!(std::fs::read_to_string(home.path().join("blocked.jsonl")).unwrap().contains("\"cleanup_timed_out\":true"));
    }
}
