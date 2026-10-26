pub mod admin;
pub mod azure;
pub mod credentials;
pub mod grants;
pub mod meta;
pub mod result;
#[cfg(feature = "desktop")]
pub mod session;
pub mod ssh;
pub mod types;
pub mod wire;

static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().thread_name("trunq-io").build().expect("tokio runtime"))
}
