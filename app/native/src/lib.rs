//! Application workflows, platform implementations and native transport.
pub mod application;
pub mod bridge;
pub mod platform;

#[cfg(all(test, not(target_os = "android")))]
#[path = "platform/android/forwarder.rs"]
mod android_forwarder_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "platform/android/capture.rs"]
mod android_capture_tests;
